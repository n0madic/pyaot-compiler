"""
A pared-down, fully-typed corpus port of Andrej Karpathy's microgpt
(gist 8627fe009c40f57531cb18360106ce95): the complete train + inference
algorithm for a tiny GPT, in the pyaot subset of Python.

Standard-syntax tweaks vs the original (behaviour-identical on both pyaot and
CPython, so the differential gate stays byte-exact):
  * the dataset is a fixed in-file list instead of a downloaded names.txt
    (drops the network branch);
  * `random.choices(list(range(n)), ...)` (list population, not a bare range);
  * `backward` uses an explicit work-stack instead of a nested recursive
    helper.
Reductions are plain `sum(...)`, `matrix` is the original lambda with a
default, and locals are unannotated — element types, sum results, and
dynamic field access are inferred/dispatched by the compiler (Phase 8H).
Everything numeric is real stdlib (`math.log/exp`, `random.gauss/shuffle/
choices` — the runtime MT19937 + libm match CPython bit-for-bit).
"""

import math
import random

random.seed(42)

docs: list[str] = [
    "emma", "olivia", "ava", "isabella", "sophia", "mia", "charlotte",
    "amelia", "evelyn", "abigail", "harper", "emily", "elizabeth", "avery",
    "sofia", "ella", "madison", "scarlett", "victoria", "aria",
]
random.shuffle(docs)
print(f"num docs: {len(docs)}")

# Tokenizer: unique characters become token ids 0..n-1, plus a BOS token.
uchars: list[str] = sorted(set("".join(docs)))
BOS = len(uchars)
vocab_size = len(uchars) + 1
print(f"vocab size: {vocab_size}")


# Autograd: recursively apply the chain rule through a computation graph.
class Value:
    __slots__ = ("data", "grad", "_children", "_local_grads")

    def __init__(self, data, children=(), local_grads=()):
        self.data = data
        self.grad = 0.0
        self._children = children
        self._local_grads = local_grads

    def __add__(self, other):
        other = other if isinstance(other, Value) else Value(other)
        return Value(self.data + other.data, (self, other), (1.0, 1.0))

    def __mul__(self, other):
        other = other if isinstance(other, Value) else Value(other)
        return Value(self.data * other.data, (self, other), (other.data, self.data))

    def __pow__(self, other):
        return Value(self.data ** other, (self,), (other * self.data ** (other - 1), ))

    def log(self):
        return Value(math.log(self.data), (self,), (1.0 / self.data, ))

    def exp(self):
        ev = math.exp(self.data)
        return Value(ev, (self,), (ev, ))

    def relu(self):
        return Value(self.data if self.data > 0.0 else 0.0, (self,), (1.0 if self.data > 0.0 else 0.0, ))

    def __neg__(self):
        return self * -1

    def __radd__(self, other):
        return self + other

    def __sub__(self, other):
        return self + (-other)

    def __rsub__(self, other):
        return other + (-self)

    def __rmul__(self, other):
        return self * other

    def __truediv__(self, other):
        return self * other ** -1

    def __rtruediv__(self, other):
        return other * self ** -1

    def backward(self):
        # Topological order via an explicit work-stack (states: 0 = entered,
        # 1 = children pushed → emit on the way up).
        topo = []
        visited = set()
        stack = [self]
        states = [0]
        while len(stack) > 0:
            top = len(stack) - 1
            node = stack[top]
            if states[top] == 0:
                if node in visited:
                    stack.pop()
                    states.pop()
                    continue
                visited.add(node)
                states[top] = 1
                kids = node._children
                for ki in range(len(kids)):
                    child = kids[ki]
                    if child not in visited:
                        stack.append(child)
                        states.append(0)
            else:
                stack.pop()
                states.pop()
                topo.append(node)
        self.grad = 1.0
        for ti in range(len(topo) - 1, -1, -1):
            v = topo[ti]
            kids2 = v._children
            grads = v._local_grads
            for ci in range(len(kids2)):
                child2 = kids2[ci]
                child2.grad = child2.grad + grads[ci] * v.grad


# Parameters. (A small config keeps the differential gate fast under the debug
# runtime; the algorithm — autograd, attention, training, sampling — is identical
# to the full-size model, and stays byte-exact vs CPython at any size.)
n_layer = 1
n_embd = 8
block_size = 8
n_head = 2
head_dim = n_embd // n_head


matrix = lambda nout, nin, std=0.08: [[Value(random.gauss(0.0, std)) for _ in range(nin)] for _ in range(nout)]


state_dict: dict[str, list[list[Value]]] = {
    "wte": matrix(vocab_size, n_embd),
    "wpe": matrix(block_size, n_embd),
    "lm_head": matrix(vocab_size, n_embd),
}
for i in range(n_layer):
    state_dict["layer" + str(i) + ".attn_wq"] = matrix(n_embd, n_embd)
    state_dict["layer" + str(i) + ".attn_wk"] = matrix(n_embd, n_embd)
    state_dict["layer" + str(i) + ".attn_wv"] = matrix(n_embd, n_embd)
    state_dict["layer" + str(i) + ".attn_wo"] = matrix(n_embd, n_embd)
    state_dict["layer" + str(i) + ".mlp_fc1"] = matrix(4 * n_embd, n_embd)
    state_dict["layer" + str(i) + ".mlp_fc2"] = matrix(n_embd, 4 * n_embd)

params: list[Value] = []
for mat in state_dict.values():
    for row in mat:
        for p in row:
            params.append(p)
print(f"num params: {len(params)}")


# Model: tokens + parameters -> logits over what comes next.
def linear(x: list[Value], w: list[list[Value]]) -> list[Value]:
    return [sum(wo[j] * x[j] for j in range(len(x))) for wo in w]


def softmax(logits: list[Value]) -> list[Value]:
    max_val = logits[0].data
    for lv in logits:
        if lv.data > max_val:
            max_val = lv.data
    exps = [(val - max_val).exp() for val in logits]
    total = sum(exps)
    return [e / total for e in exps]


def rmsnorm(x: list[Value]) -> list[Value]:
    ms = sum(xi * xi for xi in x) * (1.0 / len(x))
    scale = (ms + 1e-5) ** -0.5
    return [xi * scale for xi in x]


def gpt(token_id: int, pos_id: int,
        keys: list[list[list[Value]]], values: list[list[list[Value]]]) -> list[Value]:
    tok_emb = state_dict["wte"][token_id]
    pos_emb = state_dict["wpe"][pos_id]
    x = [tok_emb[ei] + pos_emb[ei] for ei in range(len(tok_emb))]
    x = rmsnorm(x)

    for li in range(n_layer):
        x_residual = x
        x = rmsnorm(x)
        q = linear(x, state_dict["layer" + str(li) + ".attn_wq"])
        k = linear(x, state_dict["layer" + str(li) + ".attn_wk"])
        v = linear(x, state_dict["layer" + str(li) + ".attn_wv"])
        keys[li].append(k)
        values[li].append(v)
        x_attn = []
        for h in range(n_head):
            hs = h * head_dim
            q_h = q[hs:hs + head_dim]
            k_h = [ki[hs:hs + head_dim] for ki in keys[li]]
            v_h = [vi[hs:hs + head_dim] for vi in values[li]]
            attn_logits = [
                sum(q_h[j] * k_h[t][j] for j in range(head_dim)) * (head_dim ** -0.5)
                for t in range(len(k_h))
            ]
            attn_weights = softmax(attn_logits)
            for j in range(head_dim):
                x_attn.append(sum(attn_weights[t] * v_h[t][j] for t in range(len(v_h))))
        x = linear(x_attn, state_dict["layer" + str(li) + ".attn_wo"])
        x = [x[ri] + x_residual[ri] for ri in range(len(x))]
        x_residual = x
        x = rmsnorm(x)
        x = linear(x, state_dict["layer" + str(li) + ".mlp_fc1"])
        x = [xi.relu() for xi in x]
        x = linear(x, state_dict["layer" + str(li) + ".mlp_fc2"])
        x = [x[ri2] + x_residual[ri2] for ri2 in range(len(x))]

    return linear(x, state_dict["lm_head"])


# Adam optimizer buffers.
learning_rate, beta1, beta2, eps_adam = 0.01, 0.85, 0.99, 1e-8
m: list[float] = [0.0] * len(params)
v_buf: list[float] = [0.0] * len(params)

num_steps = 2
for step in range(num_steps):
    doc = docs[step % len(docs)]
    tokens = [BOS]
    for ch in doc:
        tokens.append(uchars.index(ch))
    tokens.append(BOS)
    n = min(block_size, len(tokens) - 1)

    keys: list[list[list[Value]]] = [[] for _ in range(n_layer)]
    values: list[list[list[Value]]] = [[] for _ in range(n_layer)]
    losses = []
    for pos_id in range(n):
        token_id = tokens[pos_id]
        target_id = tokens[pos_id + 1]
        logits = gpt(token_id, pos_id, keys, values)
        probs = softmax(logits)
        losses.append(-probs[target_id].log())
    loss = sum(losses) * (1.0 / n)

    loss.backward()

    lr_t = learning_rate * (1.0 - step / num_steps)
    for i in range(len(params)):
        p = params[i]
        m[i] = beta1 * m[i] + (1.0 - beta1) * p.grad
        v_buf[i] = beta2 * v_buf[i] + (1.0 - beta2) * p.grad ** 2
        m_hat = m[i] / (1.0 - beta1 ** (step + 1))
        v_hat = v_buf[i] / (1.0 - beta2 ** (step + 1))
        p.data = p.data - lr_t * m_hat / (v_hat ** 0.5 + eps_adam)
        p.grad = 0.0

    print(f"step {step + 1:4d} / {num_steps:4d} | loss {loss.data:.4f}")

# Inference.
temperature = 0.5
print("--- inference (new, hallucinated names) ---")
for sample_idx in range(3):
    keys2: list[list[list[Value]]] = [[] for _ in range(n_layer)]
    values2: list[list[list[Value]]] = [[] for _ in range(n_layer)]
    token_id = BOS
    sample = []
    for pos_id in range(block_size):
        logits = gpt(token_id, pos_id, keys2, values2)
        scaled = [lo * (1.0 / temperature) for lo in logits]
        probs = softmax(scaled)
        weights = [pr.data for pr in probs]
        token_id = random.choices(list(range(vocab_size)), weights=weights)[0]
        if token_id == BOS:
            break
        sample.append(uchars[token_id])
    print(f"sample {sample_idx + 1:2d}: {''.join(sample)}")
