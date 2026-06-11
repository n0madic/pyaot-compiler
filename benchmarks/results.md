
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
