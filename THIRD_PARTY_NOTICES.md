# Third-party notices

## OpenAI logomark

`venus/assets/openai-blossom-104.alpha` is a rasterized mask of
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

## CodexBar CLI

The Linux Nix package includes upstream CodexBar v0.74.1, source
`7c08c0ae783b72040b728757eb85a6cfe4fc2443`, using its hash-pinned musl release
binary and resource bundle. Its MIT license is retained in
[licenses/codexbar-MIT](licenses/codexbar-MIT) and the installed package.
CodexBar owns its provider integrations; Eon exposes only normalized quota facts.

## Claude Code standalone Clawd symbol

`venus/assets/claude-code-clawd.svg` is the unaltered original
`extension/resources/clawd.svg` from Anthropic's published
[Claude Code VS Code extension](https://marketplace.visualstudio.com/items?itemName=Anthropic.claude-code),
version 2.1.323, publisher `Anthropic`.
[Original VSIX download](https://anthropic.gallery.vsassets.io/_apis/public/gallery/publisher/anthropic/extension/claude-code/2.1.323/assetbyname/Microsoft.VisualStudio.Services.VSIXPackage).
Retrieved 2026-11-13. Original SVG SHA-256:
`9ca6ebb33e268ff95ded9b927ceb53f22deecf4f1de9988f088437621c85154c`.
The containing VSIX SHA-256 is
`310d16dfb872f1f28b30189517765d790c9c077310cca55bd45b40394c1147b2`.

`venus/assets/claude-code-clawd-256.rgba` rasterizes the complete 47×38 SVG canvas,
uniformly scales it to fit 256×256 pixels, pads transparently to square and
stores it as 8-bit premultiplied RGBA. The supplied terracotta fill `#D97757`
and transparent eyes/background are retained; no path, crop or colour edit.
RGBA SHA-256:
`8f1969bb128bbc158d2b331ac4a428c9197a4abc921253f18c2d0d4cc7d5fb6b`.

The 27px and 54px Clawd PNGs rasterize the complete SVG directly at the default
1x/2x row sizes, centred on transparent square canvases. Their RGBA counterparts
store premultiplied alpha; the 256px raster serves other sizes. No small raster
is enlarged. With SIDE 27 or 54, SCALE = SIDE / 47 and TOP = SIDE * (1 - 38/47) / 2:

Exact decimal arguments used for byte-for-byte reproduction:

| SIDE | SCALE | TOP |
| --- | --- | --- |
| 27 | 0.574468085106 | 2.585106382979 |
| 54 | 1.148936170213 | 5.170212765957 |

```sh
rsvg-convert --zoom=SCALE --page-width=SIDE --page-height=SIDE --top=TOP --output=claude-code-clawd-SIDE.png claude-code-clawd.svg
magick claude-code-clawd-SIDE.png -channel RGB -fx 'u*u.a' +channel -depth 8 rgba:claude-code-clawd-SIDE.rgba
```

Preparation uses librsvg 2.62.3 and ImageMagick 7.1.2-34 Q16-HDRI. SHA-256:

| File | SHA-256 |
| --- | --- |
| `claude-code-clawd-27.png` | `1ee2ed0c8060dc40aa342f6411a4fa44ec075dcc2c84741f64ed910647554116` |
| `claude-code-clawd-27.rgba` | `a6b1a0ce1277de56209f05a9681b57fb6ed7096f4c754cd22210c85e0769f980` |
| `claude-code-clawd-54.png` | `2cc8ff31dd258113e697eff0c4357da19cf0c24daf553ce9c5dbbfe7acd37966` |
| `claude-code-clawd-54.rgba` | `7797047bc4befe0634326775ec19e54afe7f4403dee93f5529d549e00b05802d` |

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

`venus/assets/codex-symbol.svg` is the unaltered “Codex Logo - Colored” SVG from
[Zonalogo](https://zonalogo.com), downloaded and chosen by Lucca on 2026-11-13.
It is a third-party vector of OpenAI's Codex symbol, with unverified source;
the site labels its content AI-generated. It is not an official OpenAI asset.
Original SHA-256:
`983fc436a025a9920ec2661d790e956f766f273a2240801e64c3d3d26feb7ad2`.

The complete 250×250 SVG canvas retains its lilac-to-blue gradient and transparent
terminal-prompt cutout, without cropping, recolouring or redrawing. The 27px and
54px PNGs are rasterized directly at the corresponding default 1x/2x row sizes;
the RGBA files store those pixels with premultiplied alpha. The 256px RGBA
raster serves other font sizes and scales through the existing sampler.

Preparation uses `rsvg-convert` 2.62.3 (cairo 1.18.4), with this command for
SIDE 27, 54 and 256, followed by ImageMagick 7.1.2-34 Q16-HDRI:

```sh
rsvg-convert --width=SIDE --height=SIDE --output=codex-symbol-SIDE.png codex-symbol.svg
magick codex-symbol-SIDE.png -channel RGB -fx 'u*u.a' +channel -depth 8 rgba:codex-symbol-SIDE.rgba
```

SHA-256 values (paths relative to `venus/assets/`):

| File | SHA-256 |
| --- | --- |
| `codex-symbol-27.png` | `90b3eb958d0a7299d92289913411d97ded923382a4a282c27a4326069060cc73` |
| `codex-symbol-27.rgba` | `a0d99ed69ef19322844c453016f8f0583b0d941da2ae2179f607904a50acf4a6` |
| `codex-symbol-54.png` | `ebb4684136304faad4ae14ec9c8fe114d95ff24264549c56bce052fa999c5f1b` |
| `codex-symbol-54.rgba` | `71b2f6c6873009affcfbe09089644eca3531c62c63dd6c8e39a976dfb23fd927` |
| `codex-symbol-256.rgba` | `307c8d69585224de9e7d9a168916300d0ca648fece87ce74a416c419716e19e6` |

OpenAI retains its name and marks. No permissive artwork licence is claimed;
Eon's Apache-2.0 licence grants no rights to the artwork or trademarks.
[OpenAI's brand guidelines](https://openai.com/brand/) govern its marks.

By Lucca's decision of 2026-11-13, Eon uses this mark for identification only,
with the art unaltered and no implied endorsement or affiliation:

> "this is nothing for us to worry about, in practice, they add this to protect
> from people trying to use the logo for bad practices"
