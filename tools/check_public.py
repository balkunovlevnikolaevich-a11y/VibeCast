"""Reject accidentally copied private character implementations or art."""
from pathlib import Path
root=Path(__file__).resolve().parents[1]
forbidden=['backend','windows/assets/cleaners/rain-walker.png','windows/assets/cleaners/sakura-body.png','windows/assets/cleaners/sakura-head.png','core/src/simulation/character_tests.rs']
for name in forbidden:
    if (root/name).exists():raise SystemExit('Private path in public tree: '+name)
source=(root/'windows/src/render.rs').read_text(encoding='utf-8')
for text in ['fn rain_picture(', 'fn sakura_pictures(', 'sakura-head.png', 'sakura-body.png','rain-walker.png']:
    if text in source:raise SystemExit('Private renderer present: '+text)
if 'struct RainWalker' in (root/'core/src/rain_walker.rs').read_text(encoding='utf-8'):
    raise SystemExit('Private character controller present')
print('Public boundary: no Pro character implementation or art')
