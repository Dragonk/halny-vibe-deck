const path = require('path');
const fs = require('fs');

const archiver = require('archiver');

const distPath = path.join(__dirname, '..', 'dist');
const pkg = require(path.join(__dirname, '..', 'package.json'));
const zipPath = path.join(distPath, 'Halny-Vibe-Deck-' + pkg.version + '-portable.zip');
const unpackedPath = path.join(distPath, 'win-unpacked');

console.log('Creating portable zip...');

const output = fs.createWriteStream(zipPath);
const archive = archiver('zip', { zlib: { level: 9 } });

output.on('close', () => {
  console.log(`Portable zip created: ${zipPath} (${archive.pointer()} bytes)`);
});

archive.on('error', (err) => {
  throw err;
});

archive.directory(unpackedPath, false);
archive.pipe(output);
archive.finalize();
