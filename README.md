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
```

No `-o` writes `deck.pdfx.pdf` next to the input.

`--report` prints input and output sizes plus a one-line tally (orphans removed, images rewritten, duplicates dropped). If nothing got smaller you still get a file, and the report says `kept original`.

## What it does

Walks the object graph, drops unreachable objects, and collapses identical decoded images to one XObject.

RGB images that decode (raw or JPEG) get a single 4:2:0 JPEG pass. Soft masks stay DeviceGray. If a parent has to change size, the mask is resized only when nothing else points at it. Shared masks are left alone so a second image does not go transparent.

Page content streams that are already Flate stay put. An earlier pass re-encoded failed inflates as empty zlib and wiped whole slides. That is why we skip `FlateDecode` on non-image streams.

## What it will not do

It will not beat a file that is already mid-quality 4:2:0 JPEG unless we downsample. It will not touch JBIG2 or JPEG 2000. It will not decrypt anything.

## Layout

| Crate | Role |
| --- | --- |
| `pdfx-cli` | clap binary |
| `pdfx-core` | inspect / compress entry |
| `pdfx-pdf` | object graph, images, save |
| `pdfx-deflate` | zlib-wrapped DEFLATE |
| `pdfx-jpeg` | baseline SOF0 JPEG |
| `pdfx-percept` | classifier and SSIM (mostly unused on the hot path) |

```bash
cargo test --workspace
```
