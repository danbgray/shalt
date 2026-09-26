/**
 * Shalt Ikonic frontend bundle (no build step).
 * Host provides React + subsystem registry on window.
 */
const w = window;
const React = w.__IKONIC_REACT__;
const subs = w.__IKONIC_SUBSYSTEMS__ || {};
const registerSubsystem = subs.register;
const registerPanelLeafRenderer = subs.registerPanelLeafRenderer;
const DEFAULT_URL = "http://127.0.0.1:7702/?embed=1";

function resolveUrl(config) {
  const fromConfig = config && typeof config.shalt_url === "string" ? config.shalt_url : "";
  return fromConfig || DEFAULT_URL;
}

function ShaltLeaf(props) {
  const url = resolveUrl(props && props.config);
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

if (registerPanelLeafRenderer) registerPanelLeafRenderer("shalt", ShaltLeaf);

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
