const path = require("path");
const fs = require("fs");
const sharp = require("sharp");
const _pngToIco = require("png-to-ico");
const pngToIco = _pngToIco.default || _pngToIco;

const ROOT = path.join(__dirname, "..");
const svg = path.join(ROOT, "resources/halny-icon.svg");
const tmp = path.join(ROOT, ".icons-tmp");
fs.mkdirSync(tmp, { recursive: true });

async function render(size, file) {
  await sharp(svg, { density: Math.round((72 * size) / 1024) })
    .resize(size, size)
    .png({ compressionLevel: 9 })
    .toFile(file);
}

(async () => {
  for (const s of [512, 256, 128, 64, 48, 32, 16]) {
    await render(s, path.join(tmp, "icon-" + s + ".png"));
  }
  fs.copyFileSync(path.join(tmp, "icon-512.png"), path.join(ROOT, "resources/icon.png"));
  fs.copyFileSync(path.join(tmp, "icon-512.png"), path.join(ROOT, "src/renderer/public/icon.png"));
  fs.copyFileSync(path.join(tmp, "icon-256.png"), path.join(ROOT, "src/renderer/public/icon-256.png"));
  const ico = await pngToIco([256, 128, 64, 48, 32, 16].map((s) => path.join(tmp, "icon-" + s + ".png")));
  fs.writeFileSync(path.join(ROOT, "resources/icon.ico"), ico);
  fs.rmSync(tmp, { recursive: true, force: true });
  console.log(
    "ICONS DONE ico=" + fs.statSync(path.join(ROOT, "resources/icon.ico")).size +
    " png512=" + fs.statSync(path.join(ROOT, "resources/icon.png")).size +
    " png256=" + fs.statSync(path.join(ROOT, "src/renderer/public/icon-256.png")).size
  );
})().catch((e) => {
  console.error("ICONS FAIL", e && e.message ? e.message : e);
  process.exit(1);
});
