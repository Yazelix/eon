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

## Gemini mark

`assets/gemini-spark-104.alpha` is an alpha mask of
[`ProviderIcon-gemini.svg`](https://github.com/steipete/CodexBar/blob/7c08c0ae783b72040b728757eb85a6cfe4fc2443/Sources/CodexBar/Resources/ProviderIcon-gemini.svg)
from CodexBar at `7c08c0ae783b72040b728757eb85a6cfe4fc2443`, trimmed to the
spark and scaled to fill the square. The source is a single white fill; Eon
tints it Gemini blue `#4796E3`. Its MIT source license follows:

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

Google retains its Gemini name and marks. These copyright licenses do not
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

## OpenCode transparent mark

`assets/opencode-mark-dark.svg` and `assets/opencode-mark-light.svg` are the
unaltered transparent OpenCode logo variants from
[anomalyco/opencode](https://github.com/anomalyco/opencode), version 2.3.0,
commit `a6b15d33daf6ab7d35569f6d711fdb630b9f04e7`:

- `services/site/public/brand/files/opencode-logo-transparent-dark.svg`
- `services/site/public/brand/files/opencode-logo-transparent-light.svg`

Retrieved 2026-11-18. The dark-background variant retains `#EEEEEE` with
28% inner coverage; the light-background variant retains `#1F1F1F` with
20% inner coverage. The row uses the variant whose outer stroke contrasts
best with its background or highlight fill. No paths, colours or internal
opacity are edited, and no backing tile is added.

The complete 240×300 canvas is proportionally fitted into a transparent
square. Its height fills the square; equal horizontal padding centres it.
The 22/33/44px PNGs are rasterized directly at row size. Their RGBA files
store 8-bit premultiplied alpha; the 256px RGBA source serves other requested
sizes through the existing glyph-cache sampler.

Preparation uses librsvg 2.62.1 (`rsvg-convert`, cairo 1.18.4) and
ImageMagick 7.1.2-35 Q16-HDRI. For each MODE (`dark` or `light`):

| SIDE | SCALE | LEFT | TOP |
| --- | --- | --- | --- |
| 22 | 0.073333333333333 | 2.200000000000000 | 0 |
| 33 | 0.110000000000000 | 3.300000000000000 | 0 |
| 44 | 0.146666666666667 | 4.400000000000000 | 0 |
| 256 | 0.853333333333333 | 25.600000000000001 | 0 |

```sh
rsvg-convert --zoom=SCALE --page-width=SIDE --page-height=SIDE --left=LEFT --top=TOP --output=opencode-mark-MODE-SIDE.png opencode-mark-MODE.svg
magick opencode-mark-MODE-SIDE.png -channel RGB -fx 'u*u.a' +channel -depth 8 rgba:opencode-mark-MODE-SIDE.rgba
```

The 256px intermediate PNG is not shipped. SHA-256 values relative to `assets/`:

| File | SHA-256 |
| --- | --- |
| `opencode-mark-dark-22.png` | `79b327245fb40f98a1afe94fa6fe78b10efffeba5d8da38ca13cfe0b8f7b2fc8` |
| `opencode-mark-dark-22.rgba` | `b41492836b28f127f45ce399c8e2475c0521c13fd393b6bd70b83e4f09e1e965` |
| `opencode-mark-dark-256.rgba` | `77ded00108cd93ac0c80705643f5301292b02b5322ce8ac82ae112517f74fe42` |
| `opencode-mark-dark-33.png` | `551c9a4d42b66a4496d459f5cb58439d64d1c99ebe510bb8c748bb74de19e18e` |
| `opencode-mark-dark-33.rgba` | `5c5de890baed3c5efc0b31f3921b7c2503aded97d27d3854c62ef4bf41cdf5b1` |
| `opencode-mark-dark-44.png` | `d4e3d3e08e9f6083bea201f3817ec7091a6a2b06a4835f1ae3a6dfb7f7e535bf` |
| `opencode-mark-dark-44.rgba` | `73109b07d19a0db5024f981a0b25d083af03f1c2d124d8e6e24cc91855626bab` |
| `opencode-mark-dark.svg` | `502ff18efbef0432d8a702481669767c0738d744d18c45d6c3bd3b776d2d6a8e` |
| `opencode-mark-light-22.png` | `ce44b030fa00100bb95ab7ba5b314b1598cce7f81c1eb1a2b6792dffb0a78e92` |
| `opencode-mark-light-22.rgba` | `801a61405b0abd6195f323d771ffd7b118cf9d2f71598ade0ec79713344f3aa8` |
| `opencode-mark-light-256.rgba` | `cfb7698c5bfdff524e73bc1e2c27bdc3aedc5327c79833583b32c73732427024` |
| `opencode-mark-light-33.png` | `ef13165c815bf5e7af16f26f8f45f912134ddb4cc626cf4d9bbe18616f3b80c2` |
| `opencode-mark-light-33.rgba` | `9091179b0a0e13939cb4940be47f51681eb4ec715bbc8ec4badfd21c05c54670` |
| `opencode-mark-light-44.png` | `889caebbcbffef33f107fbd231e3ec8ebcdd9c679d8946793acaa5e02fb31041` |
| `opencode-mark-light-44.rgba` | `7b87f3c78c9275cd6284d1b3f351b9762845bb4156727fcf83daae5b35c52b4f` |
| `opencode-mark-light.svg` | `8fb1cffb7171c85d4c12a8219250f31a397e316b53e94ca14c171452eb3efed7` |

The upstream root `LICENSE` at that same commit applies:

```text
MIT License

Copyright (c) 2025 opencode

Permission is hereby granted, free of charge, to any person obtaining a copy
of this software and associated documentation files (the "Software"), to deal
in the Software without restriction, including without limitation the rights
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
copies of the Software, and to permit persons to whom the Software is
furnished to do so, subject to the following conditions:

The above copyright notice and this permission notice shall be included in all
copies or substantial portions of the Software.

THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
SOFTWARE.
```

OpenCode retains its name and marks; this copyright licence does not imply
affiliation or endorsement.

## Antigravity one-color icon

`assets/antigravity-icon-one-color.svg` is the unaltered official “Icon – One
Color” SVG from [Google Antigravity's press kit](https://antigravity.google/press)
([exact asset URL](https://antigravity.google/assets/image/brand/antigravity-icon__one-color.svg)).
Retrieved 2026-11-21. Original SVG SHA-256:
`5b5d0cfd07cb79ba5d299d80b754e7c91d782dc1199811688dec67a0a11ba240`.

The complete 180×180 square SVG canvas is rasterized directly without cropping,
padding changes, or path edits. The 22px, 33px, and 44px PNGs correspond to the
default 1x/1.25x, 1.5x, and 2x Agents sidebar and attention inbox row sizes;
their `.alpha` files store 8-bit single-channel alpha masks, and the 256px
`.alpha` mask serves other font sizes and scales through the existing
glyph-cache sampler. At draw time, Venus tints the mask `#FFFFFF` on dark row
surfaces and `#202124` (the vendor SVG's fill) on light row surfaces, choosing
the higher contrast against the row's resolved background.

Preparation uses `rsvg-convert` 2.62.3 (cairo 1.18.4) and ImageMagick
7.1.2-35 Q16-HDRI, for SIDE 22, 33, 44, and 256:

```sh
rsvg-convert --width=SIDE --height=SIDE --output=antigravity-icon-one-color-SIDE.png antigravity-icon-one-color.svg
magick antigravity-icon-one-color-SIDE.png -alpha extract -depth 8 gray:antigravity-icon-one-color-SIDE.alpha
```

The 256px intermediate PNG is not shipped. SHA-256 values (paths relative to
`assets/`):

| File | SHA-256 |
| --- | --- |
| `antigravity-icon-one-color.svg` | `5b5d0cfd07cb79ba5d299d80b754e7c91d782dc1199811688dec67a0a11ba240` |
| `antigravity-icon-one-color-22.png` | `9306b9be17b9dae6a11794c88fbbdd54a57030758f49001fb05eba9793b905b8` |
| `antigravity-icon-one-color-22.alpha` | `a56923f1474a7c4b9aef47ecfb9d4a5d5598f13022d25d83fd5ac2a8b1148cf8` |
| `antigravity-icon-one-color-33.png` | `53cdcbaa88da3734ac838ac7431cb7a7a2e042387fab969f597b9568142e3c38` |
| `antigravity-icon-one-color-33.alpha` | `77a707381c599518b9d682f66a2c199c9061fc89edfe0d29bca2a5cd29861df1` |
| `antigravity-icon-one-color-44.png` | `475dc1dce95cdf618ce02fc0db23e0eca94ab278c1dcc3dd008944f04e1df986` |
| `antigravity-icon-one-color-44.alpha` | `3ee07ac9dad35ad684d9d8887e0d7fb5540ba18db0b5411762eda61da9f8b5f1` |
| `antigravity-icon-one-color-256.alpha` | `b381369ef6756498e62f1d77c8484c1aa4ad70af71888e76ff874941ae1d704f` |

Google LLC retains its Antigravity name and marks. No permissive artwork licence
is claimed; Eon's Apache-2.0 licence grants no rights to Google's artwork or
trademarks. [Google Antigravity's press assets page](https://antigravity.google/press)
(read 2026-11-21) links to [Google Antigravity's Terms of Service](https://antigravity.google/terms)
(read 2026-11-21) with no permissive copyright grant. Use of Google product
icons and marks is governed by [Google's Brand Resource Center Guidance](https://about.google/brand-resource-center/guidance/),
[Product icons guidelines](https://about.google/brand-resource-center/brand-elements/#product-icons),
and [Trademark guidelines for proper usage](https://about.google/brand-resource-center/rules/)
(all read 2026-11-21), which require prior permission for product-icon use,
unaltered artwork, identifying context, and no implied endorsement or
affiliation. In Eon, the mark is shown beside the row's label in sidebar and
inbox rows—which is the program name (`Antigravity`) unless a pane pin or
record title replaces it—and never as an icon-only rail or standalone chip.

By Lucca's decision of 2026-11-13, Eon uses this mark for identification only,
with the art unaltered and no implied endorsement or affiliation:

> "this is nothing for us to worry about, in practice, they add this to protect
> from people trying to use the logo for bad practices"
