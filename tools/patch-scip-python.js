// Patches the installed scip-python so it starts on Windows (V104).
//
// scip-python 0.6.6 builds `new RegExp(path.sep, 'g')` when it loads
// (PythonEnvironment.ts). On Windows `path.sep` is a lone backslash, an
// invalid pattern, so every command, `--version` included, dies with
// "Invalid regular expression: /\/g: \ at end of pattern"
// (sourcegraph/scip-python#210; the fix, #224, is unreleased). This escapes
// the separator, which leaves the pattern unchanged where the separator is
// `/`: `\/` matches the same thing.
//
// Usage: node tools/patch-scip-python.js "$(npm root -g)"
// Idempotent. Fails when the bundle holds neither form, e.g. after an
// upgrade: drop this script once a release carries the fix.

'use strict';

const fs = require('fs');
const path = require('path');

const BROKEN = 'new RegExp(o.sep,"g")';
const FIXED = String.raw`new RegExp("\\"+o.sep,"g")`;

function count(text, needle) {
    return text.split(needle).length - 1;
}

const npmRoot = process.argv[2];
if (!npmRoot) {
    console.error('usage: node tools/patch-scip-python.js NPM_GLOBAL_ROOT');
    process.exit(2);
}
const bundle = path.join(npmRoot, '@sourcegraph', 'scip-python', 'dist', 'scip-python.js');
let text;
try {
    text = fs.readFileSync(bundle, 'utf8');
} catch (error) {
    console.error(`patch-scip-python: cannot read ${bundle}: ${error.message}`);
    process.exit(1);
}
const broken = count(text, BROKEN);
const fixed = count(text, FIXED);
if (broken === 1 && fixed === 0) {
    fs.writeFileSync(bundle, text.replace(BROKEN, FIXED));
    console.log(`patch-scip-python: patched ${bundle}`);
} else if (broken === 0 && fixed === 1) {
    console.log(`patch-scip-python: already patched: ${bundle}`);
} else {
    console.error(
        `patch-scip-python: ${bundle} holds ${broken} of \`${BROKEN}\` and ${fixed} of ` +
            `\`${FIXED}\`, expected exactly one of either; the patch is for scip-python 0.6.6 (V104)`
    );
    process.exit(1);
}
