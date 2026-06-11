
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

## 2026-06-11 18:10 — b29f0ef — 9C: MIR pipeline inline(16)+constfold+peephole+dce x2

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.724386s | 0.342978s | 0.47x |
| bench_float_kernel | 0.162747s | 0.371621s | 2.28x |
| bench_calls | 0.782281s | 0.935003s | 1.20x |
| bench_str | 0.344554s | 0.124158s | 0.36x |
| bench_containers | 0.226506s | 0.175266s | 0.77x |
| bench_exc_hotpath | 0.763516s | 0.120878s | 0.16x |
| microgpt | 1.073137s | 0.047731s | 0.04x |

## 2026-06-11 18:48 — 95b16f1 — 9 final: MIR pipeline (inline 64 + constfold/peephole/dce x2) + cold blocks + opt_level=speed + identity hash

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.725268s | 0.340404s | 0.47x |
| bench_float_kernel | 0.163930s | 0.365991s | 2.23x |
| bench_calls | 0.775945s | 0.921649s | 1.19x |
| bench_str | 0.342775s | 0.121881s | 0.36x |
| bench_containers | 0.228603s | 0.170015s | 0.74x |
| bench_exc_hotpath | 0.757103s | 0.117122s | 0.15x |
| microgpt | 0.040746s | 0.045104s | 1.11x |

## 2026-06-11 — binary size (size.sh, macOS arm64)


| artifact | full | slim |
|---|---|---|
| libpyaot_runtime.a | 32042496 | 19267728 |
| hello executable | 404808 | 355208 |
