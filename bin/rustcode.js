#!/usr/bin/env node

const fs = require('fs');
const path = require('path');
const { spawnSync } = require('child_process');

const binaryName = process.platform === 'win32' ? 'rustcode.exe' : 'rustcode';
const binaryPath = path.join(__dirname, '..', 'vendor', binaryName);

if (!fs.existsSync(binaryPath)) {
  console.error(
    'RustCode binary is missing. Reinstall the package with `npm install -g rustcode`.',
  );
  process.exit(1);
}

const result = spawnSync(binaryPath, process.argv.slice(2), { stdio: 'inherit' });

if (result.error) {
  console.error(`Failed to launch RustCode: ${result.error.message}`);
  process.exit(1);
}

process.exit(result.status === null ? 1 : result.status);
