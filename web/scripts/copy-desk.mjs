import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const here = path.dirname(fileURLToPath(import.meta.url));
const src = path.join(here, "../../crates/shalt/src/ui.html");
const dest = path.join(here, "../public/ui.html");
fs.mkdirSync(path.dirname(dest), { recursive: true });
if (fs.existsSync(src)) {
  fs.copyFileSync(src, dest);
  console.log("copied desk → public/ui.html");
} else if (fs.existsSync(dest)) {
  console.log("crates/ not in this upload; using existing public/ui.html");
} else {
  console.error("missing desk HTML at", src);
  process.exit(1);
}
