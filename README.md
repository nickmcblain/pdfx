# pdfx

CLI that shrinks PDFs. I got tired of shelling out to Ghostscript and hoping Preview still opened the file.

It only emits filters a normal viewer already knows (`FlateDecode`, `DCTDecode`). No custom codec, no qpdf, no Ghostscript. If the rewrite is not smaller, you get the original bytes back.

I wrote the DEFLATE and JPEG encoders. lopdf is parse and write only. That is slower to build and, today, the JPEG side is the weak one. Annex K tables lose to a Keynote JPEG at the same quality, so large bitmaps get boxed down to a 1920px edge at quality 62. You will see that on 4K slide photos. Text and vectors stay as they were.

Encrypted files are refused. macOS Preview wants a classic xref table, so that is what we write.

## Install

```bash
cargo install --path pdfx-cli
```

Needs Rust 1.80+.

## Usage

```bash
pdfx inspect deck.pdf
pdfx compress deck.pdf -o out.pdf --report
pdfx form application.pdf -o application.form.pdf
```

No `-o` on `compress` writes `deck.pdfx.pdf` next to the input. No `-o` on `form` writes `application.form.pdf`.

`--report` prints input and output sizes plus a one-line tally (orphans removed, images rewritten, duplicates dropped). If nothing got smaller you still get a file, and the report says `kept original`.

## What it does

Walks the object graph, drops unreachable objects, and collapses identical decoded images to one XObject.

RGB images that decode (raw or JPEG) get a single 4:2:0 JPEG pass. Soft masks stay DeviceGray. If a parent has to change size, the mask is resized only when nothing else points at it. Shared masks are left alone so a second image does not go transparent.

Page content streams that are already Flate stay put. An earlier pass re-encoded failed inflates as empty zlib and wiped whole slides. That is why we skip `FlateDecode` on non-image streams.

## Form fields

`pdfx form` does what Acrobat Pro's Prepare Form does to a flat page: it finds empty slots and drops real AcroForm widgets on them.

It looks at the content stream, not a render. A run of underscores, a horizontal rule, an empty rectangle, a small stroked square, or a ballot-box character (`☐ Coal` → checkbox `Coal`, including a Wingdings box) becomes a field. A rule with several captions on it is split, so `City:` `State:` `Zip Code:` become three fields instead of one field across the labels. Short rules and filled slivers count too (`___/___`, the blanks after `GPS Coordinates:`). Table borders are skipped. Underscore runs are measured with the page font (Helvetica, Times, Courier, and the usual aliases) so the widget sits on the blank. The name comes from the text on the left or above (`Name: ________` → `Name`). Slots with no caption are `Text1`, `Check1`, and so on. A second run leaves fields that are already there.

Text fields are `/FT /Tx`. Checkboxes are `/FT /Btn` with off-state `/Off` and on-state `/Yes`. The file sets `/NeedAppearances` so a viewer draws typed text. Filling is just setting `/V` on the widget (a PDF string for text, the name `/Yes` for a check).

Page frames, emphasis underlines, and boxes that already contain text are left alone. Encrypted files are refused.

## What it will not do

It will not beat a file that is already mid-quality 4:2:0 JPEG unless we downsample. It will not touch JBIG2 or JPEG 2000. It will not decrypt anything.

## Layout

| Crate | Role |
| --- | --- |
| `pdfx-cli` | clap binary |
| `pdfx-core` | inspect / compress / form entry |
| `pdfx-pdf` | object graph, images, save |
| `pdfx-deflate` | zlib-wrapped DEFLATE |
| `pdfx-jpeg` | baseline SOF0 JPEG |
| `pdfx-percept` | classifier and SSIM (mostly unused on the hot path) |

```bash
cargo test --workspace
```
