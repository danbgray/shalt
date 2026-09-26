# Shalt — Ikonic submodule

Shalt is a capability of Ikonic, not a separate visual language.
For Daniel and Ikonic inner-loop work, shalt replaces Hermes: the panel is the office,
`shalt play` is the factory, Gherkin is the ticket. Hermes/Claude/Grok may still *drive*
shalt via `skills/shalt`; they are not the work surface.

The Rust binary (`shalt ui`) is still the engine and the localhost API.
This package is the Ikonic glass around it: a `shalt` panel leaf and
center mode that embeds the desk at `?embed=1`. There is also a macOS
`.app` in `crates/shalt-app` — that is only a friend-share wrapper around
the same engine, not the Ikonic path.

## Load it

Ikonic discovers plugins from `{vault}/plugins/` and the in-repo `plugins/` tree.

```bash
# in-repo (this checkout)
ln -sfn /Users/danielgray/shalt/ikonic \
  /Users/danielgray/Work/Rivlet/products/ikonic/plugins/shalt

# live vault
ln -sfn /Users/danielgray/shalt/ikonic \
  ~/.ikonic/vault/plugins/shalt
```

Then restart Ikonic. Palette: **Open shalt**. Panel type: `shalt`.
Route: `/shalt`.

`config.shalt_url` defaults to `http://127.0.0.1:7702/?embed=1`.
Start the engine with `shalt ui` (or `shalt ui restart` after a rebuild).

## Theme

The embedded UI is Linear-inspired (Inter, indigo, issue rows, light/dark/blue).
Ikonic is the host shell; shalt is the panel.

Interview questions break out into a chat on **Discuss** — inline on the
ask card and as a right-hand rail. **Use this answer** copies the result
into the answer field. Continue still submits. The breakout does not write
the spec.
