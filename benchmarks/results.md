
## 2026-06-11 17:46 — f3a7982 — phase8-baseline (opt_level=none, empty optimizer pipeline)

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.726357s | 0.340724s | 0.47x |
| bench_float_kernel | 0.156083s | 0.364990s | 2.34x |
| bench_calls | 0.909142s | 0.917528s | 1.01x |
| bench_str | 0.339365s | 0.120303s | 0.35x |
| bench_containers | 0.222815s | 0.170900s | 0.77x |
| bench_exc_hotpath | 0.752746s | 0.117485s | 0.16x |
| microgpt | 1.058912s | 0.045143s | 0.04x |

## 2026-06-11 17:53 — 4797238 — 9B: cranelift opt_level=speed (default), alias-analysis on

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.730810s | 0.340839s | 0.47x |
| bench_float_kernel | 0.159485s | 0.368773s | 2.31x |
| bench_calls | 0.913092s | 0.917555s | 1.00x |
| bench_str | 0.342584s | 0.121548s | 0.35x |
| bench_containers | 0.227232s | 0.170460s | 0.75x |
| bench_exc_hotpath | 0.761143s | 0.116591s | 0.15x |
| microgpt | 1.048114s | 0.043990s | 0.04x |
