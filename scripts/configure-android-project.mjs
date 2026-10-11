import { cpSync, existsSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";

const root = fileURLToPath(new URL("../", import.meta.url));
const overlay = join(root, "src-tauri", "android");
const generated = join(root, "src-tauri", "gen", "android");

if (!existsSync(generated)) {
  throw new Error(
    "Generated Android project not found at src-tauri/gen/android. Run `npm run tauri:android:init` first.",
  );
}

if (!existsSync(overlay)) {
  console.log("No maintained src-tauri/android overlay found; generated Android project is unchanged.");
  process.exit(0);
}

cpSync(overlay, generated, {
  recursive: true,
  force: true,
  errorOnExist: false,
});
console.log("Applied maintained src-tauri/android overlay to src-tauri/gen/android.");
