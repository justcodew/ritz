"""命令行 smoke test：python -m ritz -- file.pdf"""

import sys
import argparse


def main(argv=None):
    parser = argparse.ArgumentParser(prog="ritz", description="ritz PDF smoke test")
    parser.add_argument("file", help="PDF file path")
    parser.add_argument("--dpi", type=int, default=72, help="render DPI")
    parser.add_argument("--text", default="text", help="text mode: text/html/xml")
    args = parser.parse_args(argv)

    import ritz

    doc = ritz.open(args.file)
    print(f"file: {args.file}")
    print(f"page_count: {doc.page_count}")
    print(f"format: {doc.metadata('format')}")

    page = doc[0]
    print(f"page 0 rect: {page.rect}")
    print(f"page 0 size: {page.width:.1f} x {page.height:.1f}")

    text = page.get_text(args.text)
    preview = text[:200].replace("\n", " ")
    print(f"text ({args.text}, {len(text)} chars): {preview!r}")

    pix = page.get_pixmap(dpi=args.dpi)
    print(f"pixmap: {pix.width} x {pix.height}, stride={pix.stride}, n={pix.components}")

    png = pix.tobytes("png")
    print(f"png bytes: {len(png)}")


if __name__ == "__main__":
    main()
