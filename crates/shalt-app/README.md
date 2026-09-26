# Shalt.app

A friend-share wrapper: the same `shalt ui` desk in a native window.

This is **not** the inner-loop product. Ikonic loads shalt as a submodule
(`ikonic/` at the repo root). `shalt ui` is the engine both surfaces talk to.

```
../../scripts/macos-install.sh
open -a Shalt
shalt --help
```

The installer (and first launch) copies the bundled `shalt` CLI onto PATH.
