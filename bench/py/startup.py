# Startup smoke test — smallest meaningful program.
# Run-time here measures binary launch + single print; end-to-end measures
# compile+launch. Regressions in either stage surface as jumps in the
# `startup` bench line.

def main() -> None:
    print("startup: ok")


if __name__ == "__main__":
    main()
