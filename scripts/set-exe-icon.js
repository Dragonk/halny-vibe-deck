const { execSync } = require('child_process');
const path = require('path');

const rcedit = path.join(__dirname, '..', 'node_modules', 'rcedit', 'bin', 'rcedit.exe');
const exePath = path.join(__dirname, '..', 'dist', 'win-unpacked', 'Mistral Vibe Nexus.exe');
const iconPath = path.join(__dirname, '..', 'resources', 'icon.ico');

console.log('Setting icon on executable...');
execSync(`"${rcedit}" "${exePath}" --set-icon "${iconPath}"`, { stdio: 'inherit' });
console.log('Icon set successfully!');
