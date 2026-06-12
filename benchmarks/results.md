
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

## 2026-06-11 20:41 — 7770f1c — phase3c-raw-int

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.717110s | 0.338049s | 0.47x |
| bench_float_kernel | 0.159214s | 0.364347s | 2.29x |
| bench_calls | 0.568831s | 0.914232s | 1.61x |
| bench_str | 0.337062s | 0.118460s | 0.35x |
| bench_containers | 0.093405s | 0.169267s | 1.81x |
| bench_exc_hotpath | 0.627176s | 0.116622s | 0.19x |
| microgpt | 0.040722s | 0.044863s | 1.10x |

## 2026-06-11 23:52 — 0373ba2 — cached-charlen

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.724764s | 0.342219s | 0.47x |
| bench_float_kernel | 0.156409s | 0.374029s | 2.39x |
| bench_calls | 0.576996s | 0.928673s | 1.61x |
| bench_str | 0.242066s | 0.120422s | 0.50x |
| bench_containers | 0.100211s | 0.171625s | 1.71x |
| bench_exc_hotpath | 0.632070s | 0.118084s | 0.19x |
| microgpt | 0.041095s | 0.045740s | 1.11x |

## 2026-06-12 01:39 — d077a3e — table-unwinding

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.735292s | 0.339220s | 0.46x |
| bench_float_kernel | 0.156167s | 0.362431s | 2.32x |
| bench_calls | 0.566472s | 0.915726s | 1.62x |
| bench_str | 0.230776s | 0.117939s | 0.51x |
| bench_containers | 0.097907s | 0.169309s | 1.73x |
| bench_exc_hotpath | 0.286616s | 0.115857s | 0.40x |
| microgpt | 0.039816s | 0.044601s | 1.12x |

## 2026-06-12 09:05 — 9db2a71 — real-tracebacks

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.725308s | 0.344865s | 0.48x |
| bench_float_kernel | 0.156251s | 0.366695s | 2.35x |
| bench_calls | 0.572772s | 0.928747s | 1.62x |
| bench_str | 0.234721s | 0.119363s | 0.51x |
| bench_containers | 0.099412s | 0.172583s | 1.74x |
| bench_exc_hotpath | 0.288923s | 0.118514s | 0.41x |
| microgpt | 0.040658s | 0.044952s | 1.11x |

## 2026-06-12 10:14 — b87168d — PLAN#7: interproc raw-int + str ASCII case

| bench | pyaot | cpython | ratio (cpython/pyaot) |
|---|---|---|---|
| bench_int_loop | 0.668303s | 0.341237s | 0.51x |
| bench_float_kernel | 0.151591s | 0.366693s | 2.42x |
| bench_calls | 0.567630s | 0.924317s | 1.63x |
| bench_str | 0.111420s | 0.116816s | 1.05x |
| bench_containers | 0.098129s | 0.171591s | 1.75x |
| bench_exc_hotpath | 0.143464s | 0.117930s | 0.82x |
| microgpt | 0.040027s | 0.044985s | 1.12x |
