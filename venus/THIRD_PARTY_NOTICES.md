# Third-party notices

## OpenAI logomark

`assets/openai-blossom-104.alpha` is a rasterized mask of
`examples/agents_sdk/deployment_manager/frontend/src/openai-logomark.svg` from
[`openai/openai-cookbook`](https://github.com/openai/openai-cookbook) at commit
`263b2d5b7b63836c4ea30ee9709e65b7a50cbf6f`. The upstream file is
provided under the following MIT license:

> MIT License
>
> Copyright (c) 2025 OpenAI
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

OpenAI retains its marks. Eon's Apache-2.0 license does not grant
rights to the OpenAI name or Blossom as trademarks. Their use remains subject
to [OpenAI's brand guidelines](https://openai.com/brand/).

## Claude logomark

`assets/claude-splat-104.alpha` is an alpha mask of
[`Brand-ProviderIcon-claude.svg`](https://github.com/steipete/CodexBar/blob/7c08c0ae783b72040b728757eb85a6cfe4fc2443/Sources/CodexBar/Resources/Brand-ProviderIcon-claude.svg)
from CodexBar at `7c08c0ae783b72040b728757eb85a6cfe4fc2443`. The original
terracotta fill is `#D97757`. Its MIT source license follows:

> MIT License
>
> Copyright (c) 2026 Peter Steinberger
>
> Permission is hereby granted, free of charge, to any person obtaining a copy
> of this software and associated documentation files (the "Software"), to deal
> in the Software without restriction, including without limitation the rights
> to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
> copies of the Software, and to permit persons to whom the Software is
> furnished to do so, subject to the following conditions:
>
> The above copyright notice and this permission notice shall be included in all
> copies or substantial portions of the Software.
>
> THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
> IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
> FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
> AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
> LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
> OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
> SOFTWARE.

Anthropic retains its Claude name and marks. These copyright licenses do not
grant trademark rights or imply affiliation or endorsement.

## Claude Code standalone Clawd symbol

`assets/claude-code-clawd.svg` is the unaltered original
`extension/resources/clawd.svg` from Anthropic's published
[Claude Code VS Code extension](https://marketplace.visualstudio.com/items?itemName=Anthropic.claude-code),
version 2.1.323, publisher `Anthropic`.
[Original VSIX download](https://anthropic.gallery.vsassets.io/_apis/public/gallery/publisher/anthropic/extension/claude-code/2.1.323/assetbyname/Microsoft.VisualStudio.Services.VSIXPackage).
Retrieved 2026-11-13. Original SVG SHA-256:
`9ca6ebb33e268ff95ded9b927ceb53f22deecf4f1de9988f088437621c85154c`.
The containing VSIX SHA-256 is
`310d16dfb872f1f28b30189517765d790c9c077310cca55bd45b40394c1147b2`.

`assets/claude-code-clawd-256.rgba` rasterizes the complete 47×38 SVG canvas,
uniformly scales it to fit 256×256 pixels, pads transparently to square and
stores it as 8-bit premultiplied RGBA. The supplied terracotta fill `#D97757`
and transparent eyes/background are retained; no path, crop or colour edit.
RGBA SHA-256:
`8f1969bb128bbc158d2b331ac4a428c9197a4abc921253f18c2d0d4cc7d5fb6b`.

The 22px, 33px and 44px Clawd PNGs proportionally scale the unedited SVG to
whole-pixel column starts, with the sub-pixel edge trim described below.
The source's eleven column starts have a
4.1511431-unit pitch. Each pitch occupies 2, 3 or 4 pixels, respectively;
the first column starts at x=0. The original overlapping rectangles extend
past that grid: the right canvas edge trims 0.068/0.102/0.136 pixels of painted
fringe. Colours, paths and internal transparency are unchanged. Horizontal
edges retain the SVG's fractional vertical pitch; this is not a claim that
all artwork edges land on whole pixels. The art top is rounded to a whole
pixel near vertical centre. RGBA files store premultiplied alpha.

Default 1x and 1.25x use 22px, 1.5x uses 33px and 2x uses 44px. Other requested
sizes use the existing cache-time sampler from the complete 256px raster;
those sizes can be softer. No bitmap is enlarged or scaled while drawing.

Exact decimal arguments for byte-for-byte reproduction:

| SIDE | SCALE | LEFT | TOP |
| --- | --- | --- | --- |
| 22 | 0.481795002442 | -0.448439852628 | 1.547854180214 |
| 33 | 0.722692503662 | -0.672659778941 | 2.321781270320 |
| 44 | 0.963590004883 | -0.896879705255 | 4.095708360427 |

```sh
rsvg-convert --zoom=SCALE --page-width=SIDE --page-height=SIDE --left=LEFT --top=TOP --output=claude-code-clawd-SIDE.png claude-code-clawd.svg
magick claude-code-clawd-SIDE.png -channel RGB -fx 'u*u.a' +channel -depth 8 rgba:claude-code-clawd-SIDE.rgba
```

Preparation uses librsvg 2.62.3 and ImageMagick 7.1.2-34 Q16-HDRI. SHA-256:

| File | SHA-256 |
| --- | --- |
| `claude-code-clawd-22.png` | `40412285c4931e7df5193473cd301f146aa23cd373e0bbfd2ea3e26e068d35e2` |
| `claude-code-clawd-22.rgba` | `862167f874de6e3a2348dd2cea1cdb7add3afe10187816b748aa092e39972d64` |
| `claude-code-clawd-33.png` | `8ec85689732f93a5cec80fe8d637db7f697832425a0b6f9cada289e3fbcccf41` |
| `claude-code-clawd-33.rgba` | `c3d8c916c4efb305d305d2319e14c06f15e275f9f7ff1a4b47958bc72c993d51` |
| `claude-code-clawd-44.png` | `2a118cb1a7e2ec3560dd4752cd4302ad17bc17b725118bd6c99db6292d09bc7c` |
| `claude-code-clawd-44.rgba` | `0355088c462c27ec5fe81cdfa91b4771739e37cea08d9e617726fe9581e8cbf4` |

Copyright Anthropic PBC. No copyright licence; used under the vendor's brand
terms. The extension reserves Anthropic's rights and points to its
[Claude Code legal agreements](https://code.claude.com/docs/en/legal-and-compliance).
[Anthropic's Trademark Guidelines](https://www.anthropic.com/legal/trademark-guidelines)
(read 2026-11-13) require prior approval, supplied unaltered art, readable
placement and spacing, and no implied endorsement or affiliation; permission
may be terminated. These sources are not a permissive copyright licence or
an independent redistribution grant. Eon's Apache-2.0 licence grants no
rights to Anthropic's artwork or trademarks.

By Lucca's decision of 2026-11-13, Eon uses this mark for identification only,
with the art unaltered and no implied endorsement or affiliation:

> "this is nothing for us to worry about, in practice, they add this to protect
> from people trying to use the logo for bad practices"


## Codex colour symbol

`assets/codex-symbol.svg` is the unaltered “Codex Logo - Colored” SVG from
[Zonalogo](https://zonalogo.com), downloaded and chosen by Lucca on 2026-11-13.
It is a third-party vector of OpenAI's Codex symbol, with unverified source;
the site labels its content AI-generated. It is not an official OpenAI asset.
Original SHA-256:
`983fc436a025a9920ec2661d790e956f766f273a2240801e64c3d3d26feb7ad2`.

The complete 250×250 SVG canvas retains its lilac-to-blue gradient and transparent
terminal-prompt cutout, without cropping, recolouring or redrawing. The 22px, 33px and
44px PNGs are rasterized directly at the corresponding default 1x/1.25x, 1.5x and 2x row sizes;
the RGBA files store those pixels with premultiplied alpha. The 256px RGBA
raster serves other font sizes and scales through the existing sampler.

Preparation uses `rsvg-convert` 2.62.3 (cairo 1.18.4), with this command for
SIDE 22, 33, 44 and 256, followed by ImageMagick 7.1.2-34 Q16-HDRI:

```sh
rsvg-convert --width=SIDE --height=SIDE --output=codex-symbol-SIDE.png codex-symbol.svg
magick codex-symbol-SIDE.png -channel RGB -fx 'u*u.a' +channel -depth 8 rgba:codex-symbol-SIDE.rgba
```

SHA-256 values (paths relative to `assets/`):

| File | SHA-256 |
| --- | --- |
| `codex-symbol-22.png` | `2184ee35861e8592e6fb2fb552f9b0ea5f851629d5dbaee02eec19f59d950ec5` |
| `codex-symbol-22.rgba` | `d16e931ac2f7194bc1dedb3a0b8a8dd46f87a725b20bc2f5d1582188386afa48` |
| `codex-symbol-33.png` | `6351a75207b254491927dc3ec5d6d5fb9c9a7ba10b3d883edaecca12a8c1c5cf` |
| `codex-symbol-33.rgba` | `a7d821a4272dd9500a05a098b883e5d62d93d952e589c457acde0d397ed36a1c` |
| `codex-symbol-44.png` | `9089fbeb8f91931754cd3fbe6d64dd5860ff4adf86e75dd45a9e6b9519008b5d` |
| `codex-symbol-44.rgba` | `7be74d8cf5b1c39500867ce3e352f52d8707641e941dabd0dabeb3724606f9ea` |
| `codex-symbol-256.rgba` | `307c8d69585224de9e7d9a168916300d0ca648fece87ce74a416c419716e19e6` |

OpenAI retains its name and marks. No permissive artwork licence is claimed;
Eon's Apache-2.0 licence grants no rights to the artwork or trademarks.
[OpenAI's brand guidelines](https://openai.com/brand/) govern its marks.

By Lucca's decision of 2026-11-13, Eon uses this mark for identification only,
with the art unaltered and no implied endorsement or affiliation:

> "this is nothing for us to worry about, in practice, they add this to protect
> from people trying to use the logo for bad practices"
