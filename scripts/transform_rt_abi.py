#!/usr/bin/env python3
"""
Two-layer ABI transformation for rt_* extern "C" functions in crates/runtime/src/.

Strategy: for each `#[no_mangle] extern "C" fn rt_foo(a: *mut Obj, ...) -> *mut Obj`:
  1. Remove #[no_mangle] and `extern "C"` → becomes `pub fn rt_foo(a: *mut Obj, ...) -> *mut Obj`
     (body is 100% unchanged — zero internal call-site edits needed)
  2. Insert after the closing `}`:
       #[export_name = "rt_foo"]
       #[allow(clippy::not_unsafe_ptr_arg_deref)]
       pub extern "C" fn rt_foo_abi(a: Value, ...) -> Value {
           Value::from_ptr(rt_foo(a.unwrap_ptr(), ...))
       }

Internal callers continue using `rt_foo(*mut Obj)` — no edits required.
Compiled Python code links against the C symbol `rt_foo` → resolved to the thin wrapper.

Usage:
  python3 scripts/transform_rt_abi.py [--dry-run] [file ...]
  With no file args: processes all *.rs under crates/runtime/src/.
"""

import re
import sys
from pathlib import Path

ROOT = Path(__file__).parent.parent
RUNTIME_SRC = ROOT / "crates/runtime/src"

# ── Regex helpers ─────────────────────────────────────────────────────────────

# Matches #[no_mangle] extern "C" fn rt_* functions (with optional preceding attributes)
EXTERN_RT_RE = re.compile(
    r'(?P<attrs>(?:#\[[^\]]*\]\s*)*)'        # zero or more attributes
    r'(?P<vis>pub\s+)?'
    r'(?P<unsf>unsafe\s+)?'
    r'extern\s+"C"\s+'
    r'fn\s+(?P<name>rt_\w+)'
    r'\s*\((?P<params>[^)]*)\)'              # param list (no nested parens needed for C ABI)
    r'(?:\s*->\s*(?P<ret>[^{;]+?))?'         # optional return type
    r'\s*\{'                                 # opening brace
)

# Matches a bare `*mut Obj` parameter (not *mut StrObj etc.)
OBJ_PARAM_RE = re.compile(r'(\w+)\s*:\s*(?:(?:mut\s+)?\*\s*mut\s+Obj\b)')

# Matches a return type that is exactly `*mut Obj`
RETURNS_OBJ_RE = re.compile(r'^\s*\*\s*mut\s+Obj\s*$')

NO_MANGLE_ATTR = re.compile(r'#\[no_mangle\]\s*')


# ── Import helpers ────────────────────────────────────────────────────────────

def has_value_import(content: str) -> bool:
    """True if the file already has a top-level `use ... Value` import."""
    return bool(re.search(
        r'^(?:pub\s+)?use\s+(?:pyaot_core_defs|crate::object|crate::value)'
        r'\s*::\s*(?:Value\b|\{[^}]*\bValue\b)',
        content, re.MULTILINE,
    ))


def has_serde_value_import(content: str) -> bool:
    """True if file imports serde_json::Value — conflicts with our Value."""
    return bool(re.search(
        r'use\s+serde_json\s*::\s*(?:\{[^}]*\bValue\b|Value\b)', content
    ))


def add_value_import(content: str) -> str:
    """Insert `use pyaot_core_defs::Value;` after the last existing pyaot_core_defs import line."""
    m = list(re.finditer(r'^use\s+pyaot_core_defs\s*::[^\n]+\n', content, re.MULTILINE))
    if m:
        ins = m[-1].end()
        return content[:ins] + 'use pyaot_core_defs::Value;\n' + content[ins:]
    m = list(re.finditer(r'^use\s+crate\s*::[^\n]+\n', content, re.MULTILINE))
    if m:
        ins = m[-1].end()
        return content[:ins] + 'use pyaot_core_defs::Value;\n' + content[ins:]
    m = re.search(r'^use\s+', content, re.MULTILINE)
    if m:
        lines = content.split('\n')
        idx = content[:m.start()].count('\n')
        while idx < len(lines) and lines[idx].startswith('use '):
            idx += 1
        ins = sum(len(l) + 1 for l in lines[:idx])
        return content[:ins] + 'use pyaot_core_defs::Value;\n' + content[ins:]
    return content


# ── Brace matcher ─────────────────────────────────────────────────────────────

def find_matching_brace(text: str, open_pos: int) -> int:
    """Given the position of `{`, return the position of the matching `}`."""
    depth = 0
    i = open_pos
    in_lc = in_bc = in_str = in_ch = esc = False
    while i < len(text):
        c = text[i]
        if esc:
            esc = False; i += 1; continue
        if in_lc:
            if c == '\n': in_lc = False
            i += 1; continue
        if in_bc:
            if text[i:i+2] == '*/': in_bc = False; i += 2
            else: i += 1
            continue
        if in_str:
            if c == '\\': esc = True
            elif c == '"': in_str = False
            i += 1; continue
        if in_ch:
            if c == '\\': esc = True
            elif c == "'": in_ch = False
            i += 1; continue
        if text[i:i+2] == '//': in_lc = True; i += 2; continue
        if text[i:i+2] == '/*': in_bc = True; i += 2; continue
        if c == '"': in_str = True; i += 1; continue
        if c == "'": in_ch = True; i += 1; continue
        if c == '{': depth += 1
        elif c == '}':
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return -1


# ── Param helpers ─────────────────────────────────────────────────────────────

def extract_obj_param_names(params_str: str) -> list[str]:
    """Return names of params typed `*mut Obj` (bare Obj only)."""
    return [m.group(1) for m in OBJ_PARAM_RE.finditer(params_str)]


def transform_params_str(params_str: str) -> str:
    """Replace `name: *mut Obj` → `name: Value` and strip leading `mut` binding modifiers."""
    result = OBJ_PARAM_RE.sub(lambda m: f'{m.group(1)}: Value', params_str)
    # Strip binding `mut` from each param (mut is on the Value binding, not needed in wrapper)
    result = re.sub(r'(?<![*\w])mut\s+(\w+)\s*:', r'\1:', result)
    return result


def build_call_args(params_str: str, obj_names: set[str]) -> str:
    """
    Build the call argument list for the wrapper → internal function call.
    *mut Obj params are unwrapped via `.unwrap_ptr()`; others pass through by name.
    """
    if not params_str.strip():
        return ''
    args = []
    for part in params_str.split(','):
        part = part.strip()
        if not part:
            continue
        # Handle optional leading `mut` binding modifier: `mut key: T`
        m = re.match(r'(?:mut\s+)?(\w+)\s*:', part)
        if m:
            pname = m.group(1)
            if pname in obj_names:
                args.append(f'{pname}.unwrap_ptr()')
            else:
                args.append(pname)
        else:
            args.append(part)
    return ', '.join(args)


# ── Core transformation ───────────────────────────────────────────────────────

def transform_function(content: str, m: re.Match) -> tuple[str, int] | None:
    """
    Transform one rt_* extern "C" function matched by `m` in `content`.

    Returns (replacement_text, end_pos_in_content) where:
      - replacement_text replaces content[m.start() : end_pos]
      - end_pos is just past the closing `}` of the original function

    Returns None if the function needs no transformation.
    """
    name = m.group('name')
    params_str = m.group('params')
    ret_str = (m.group('ret') or '').strip()
    attrs = m.group('attrs') or ''
    vis = m.group('vis') or ''
    unsf = m.group('unsf') or ''

    obj_params = extract_obj_param_names(params_str)
    returns_obj = bool(RETURNS_OBJ_RE.match(ret_str)) if ret_str else False

    if not obj_params and not returns_obj:
        return None

    # Find the function body
    brace_pos = content.find('{', m.end() - 1)  # m ends just past `{`
    if brace_pos == -1:
        return None
    close_pos = find_matching_brace(content, brace_pos)
    if close_pos == -1:
        return None

    body_and_close = content[brace_pos + 1 : close_pos + 1]  # body + closing }

    # ── Internal function: remove #[no_mangle], remove `extern "C"` ──────────

    internal_attrs = NO_MANGLE_ATTR.sub('', attrs)

    ret_annotation = f' -> {ret_str}' if ret_str else ''
    internal_sig = f'{internal_attrs}{vis}{unsf}fn {name}({params_str}){ret_annotation} {{'
    internal_fn = internal_sig + body_and_close  # body includes closing }

    # ── ABI wrapper ───────────────────────────────────────────────────────────

    new_params = transform_params_str(params_str)
    new_ret = ' -> Value' if returns_obj else (f' -> {ret_str}' if ret_str else '')

    obj_set = set(obj_params)
    call_args = build_call_args(params_str, obj_set)
    internal_call = f'{name}({call_args})'
    if unsf:
        internal_call = f'unsafe {{ {internal_call} }}'
    wrapper_body = f'Value::from_ptr({internal_call})' if returns_obj else internal_call

    wrapper_name = f'{name}_abi'

    # Carry over any #[cfg(...)] attributes for conditional compilation
    cfg_attrs = ''.join(re.findall(r'#\[cfg[^\]]*\]\s*', attrs))

    # Don't double-add #[allow(clippy::not_unsafe_ptr_arg_deref)] if already present
    has_allow = '#[allow(clippy::not_unsafe_ptr_arg_deref)]' in internal_attrs
    allow_line = '' if has_allow else '#[allow(clippy::not_unsafe_ptr_arg_deref)]\n'

    wrapper = (
        f'\n{cfg_attrs}'
        f'#[export_name = "{name}"]\n'
        f'{allow_line}'
        f'pub extern "C" fn {wrapper_name}({new_params}){new_ret} {{\n'
        f'    {wrapper_body}\n'
        f'}}\n'
    )

    replacement = internal_fn + wrapper
    return replacement, close_pos + 1


def process_content(content: str) -> tuple[str, int]:
    """Transform all extern "C" rt_* functions. Returns (new_content, n_changed)."""
    # Skip files with serde_json::Value to avoid name collision
    if has_serde_value_import(content):
        return content, 0

    parts: list[str] = []
    pos = 0
    changes = 0

    while pos < len(content):
        m = EXTERN_RT_RE.search(content, pos)
        if not m:
            parts.append(content[pos:])
            break

        func_start = m.start()

        result = transform_function(content, m)
        if result is None:
            # No transformation; emit up to and including the function body
            brace_pos = content.find('{', m.end() - 1)
            if brace_pos == -1:
                parts.append(content[pos:])
                break
            close_pos = find_matching_brace(content, brace_pos)
            if close_pos == -1:
                parts.append(content[pos:])
                break
            parts.append(content[pos : close_pos + 1])
            pos = close_pos + 1
            continue

        replacement, end_pos = result
        parts.append(content[pos : func_start])  # text before this function
        parts.append(replacement)
        pos = end_pos
        changes += 1

    new_content = ''.join(parts)

    if changes > 0 and not has_value_import(new_content):
        new_content = add_value_import(new_content)

    return new_content, changes


def process_file(path: Path, dry_run: bool = False) -> bool:
    content = path.read_text()
    new_content, changes = process_content(content)
    if changes == 0 or new_content == content:
        return False
    if dry_run:
        print(f'[dry-run] would change: {path}  ({changes} functions)')
        return True
    path.write_text(new_content)
    print(f'Changed: {path}  ({changes} functions)')
    return True


def main():
    args = sys.argv[1:]
    dry_run = '--dry-run' in args
    args = [a for a in args if a != '--dry-run']

    files = [Path(a) for a in args] if args else list(RUNTIME_SRC.rglob('*.rs'))

    changed = 0
    for f in sorted(files):
        if process_file(f, dry_run=dry_run):
            changed += 1

    print(f'\nTotal files changed: {changed}')


if __name__ == '__main__':
    main()
