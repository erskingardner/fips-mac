# FIPS Mac Icon Composer artwork

The mark is an `F` built from a small network topology: one backbone, two
routes, and a brighter live endpoint. It stays recognizable at menu-bar and
Finder sizes without falling back to a generic shield or lock.

## Import

1. Create a new Mac icon in Icon Composer with a 1024 x 1024 canvas.
2. Drag the three numbered folders into the sidebar together. Their names keep
   the groups in back-to-front order.
3. Add the background in Icon Composer instead of importing a background SVG.
4. Use `#10251F` at the upper-left and `#06100D` at the lower-right for the
   default appearance. A solid `#0A1814` also works well.
5. Keep the rails relatively quiet, give Nodes a little more depth, and make
   Live the brightest/frontmost group. Start with subtle automatic specular and
   shadow settings; the geometry should remain the main effect.

Suggested appearance colors:

| Appearance | Background | Rails | Nodes | Live |
| --- | --- | --- | --- | --- |
| Default | `#10251F` to `#06100D` | `#59E5B1` | `#75F0C1` | `#B8FFE4` |
| Dark | `#07110F` | `#42C995` | `#59E5B1` | `#A9FFDD` |
| Mono | system-controlled | white | white | white |

`fips-mac-composite.svg` is a flat reference preview, not an import layer.
It intentionally has no rounded mask because the system applies the final crop.
