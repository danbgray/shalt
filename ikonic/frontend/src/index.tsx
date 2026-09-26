/**
 * Shalt as an Ikonic submodule.
 *
 * Registers a `shalt` center mode + panel leaf that embeds the local
 * `shalt ui` (LCARS-themed) at the configured URL. The Rust binary remains
 * the engine; this surface is the Ikonic glass around it.
 *
 * Host globals (set by Ikonic before the bundle loads):
 *   window.__IKONIC_REACT__
 *   window.__IKONIC_SUBSYSTEMS__.register / registerPanelLeafRenderer
 */

const w = window as any;
const React = w.__IKONIC_REACT__;
const { register: registerSubsystem, registerPanelLeafRenderer } = w.__IKONIC_SUBSYSTEMS__ || {};

const DEFAULT_URL = "http://127.0.0.1:7702/?embed=1";

function resolveUrl(config?: Record<string, unknown>): string {
  const fromConfig = typeof config?.shalt_url === "string" ? config.shalt_url : "";
  return fromConfig || DEFAULT_URL;
}

function ShaltLeaf(props: { config?: Record<string, unknown> }) {
  const url = resolveUrl(props.config);
  return React.createElement("iframe", {
    src: url,
    title: "shalt",
    style: {
      width: "100%",
      height: "100%",
      border: 0,
      background: "transparent",
      display: "block",
    },
  });
}

if (registerPanelLeafRenderer) {
  registerPanelLeafRenderer("shalt", ShaltLeaf);
}

if (registerSubsystem) {
  registerSubsystem({
    name: "shalt",
    version: "0.1.0",
    description: "English → Gherkin → tests → code",
    color: "gold",
    dependencies: [],
    centerModes: {
      shalt: {
        urlPattern: "/shalt",
        buildConfig: () => ({}),
      },
    },
    renderers: { shalt: ShaltLeaf },
    leafRenderers: { shalt: ShaltLeaf },
    commandPaletteItems: [
      {
        id: "open-shalt",
        label: "Open shalt",
        keywords: ["shalt", "spec", "gherkin", "bdd", "author"],
        action: () => {
          window.dispatchEvent(
            new CustomEvent("ikonic:navigate", { detail: { mode: "shalt" } }),
          );
        },
      },
    ],
  });
}

console.log("[shalt] Ikonic submodule loaded");
export default { name: "shalt" };
