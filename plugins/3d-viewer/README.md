# 3D Viewer

Views self-contained 3D assets (`.glb`, `.obj`) via [three.js](https://threejs.org/), vendored
from the npm package (r0.185.1, MIT — see `LICENSE.md`).

## Notes

No external references yet — a loose `.gltf` pointing at separate `.bin`/texture files, or an
`.obj`'s `mtllib`, isn't resolved. A self-contained `.glb` (glTF binary, everything embedded) or
a plain `.obj` with no external material file both work today.
