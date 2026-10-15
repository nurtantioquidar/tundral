#!/usr/bin/env python3
"""Regenerate the binary inputs of the semantic fixture (PDF, PNG, DOCX, XLSX, WAV).

    uv run --no-project bench/semantic/make_binaries.py cli/tests/fixtures/semantic/mixed

Stdlib only, deterministic bytes (fixed timestamps). The PNG is rendered from a one-page PDF
with macOS `sips`; elsewhere pass `--no-png` and keep the committed PNG.
"""

from __future__ import annotations

import argparse
import io
import math
import struct
import subprocess
import tempfile
import wave
import zipfile
from pathlib import Path

PAPER = [
    ("F2", 16, "Write-Through Caching for Job Queues"),
    ("F1", 11, "A. Example, B. Sample - Monet Labs, 2026"),
    ("F2", 12, "Abstract"),
    (
        "F1",
        10,
        "We study a write-through cache placed in front of a durable job queue. Each",
    ),
    (
        "F1",
        10,
        "job is pushed through JobQueue.push and slugified before storage. The cache",
    ),
    (
        "F1",
        10,
        "reduces read latency by 41 percent on the QueueBench workload while keeping",
    ),
    (
        "F1",
        10,
        "exactly-once delivery. We compare it with the lazy invalidation scheme of",
    ),
    ("F1", 10, "Smith et al. [1] and show that write-through avoids stale reads."),
    ("F2", 12, "References"),
    ("F1", 10, "[1] J. Smith, L. Wong. Lazy Invalidation in Distributed Caches. 2019."),
]

DIAGRAM = [
    ("F2", 14, "Architecture"),
    ("F1", 12, "[ README ] --> [ run_job ] --> [ JobQueue ]"),
    ("F1", 12, "                               |"),
    ("F1", 12, "                               v"),
    ("F1", 12, "                         [ slugify ] --> [ Cache ]"),
]


def pdf(
    lines: list[tuple[str, int, str]],
    width: int = 612,
    height: int = 792,
    background: bool = False,
) -> bytes:
    y = height - 72
    # A filled white page keeps the rendered PNG opaque (a transparent one reads as black).
    ops = [f"1 1 1 rg 0 0 {width} {height} re f 0 0 0 rg"] if background else []
    for font, size, text in lines:
        esc = text.replace("\\", "\\\\").replace("(", "\\(").replace(")", "\\)")
        ops.append(f"BT /{font} {size} Tf 72 {y} Td ({esc}) Tj ET")
        y -= int(size * 1.8)
    stream = "\n".join(ops).encode()
    objs = [
        b"<< /Type /Catalog /Pages 2 0 R >>",
        b"<< /Type /Pages /Kids [3 0 R] /Count 1 >>",
        f"<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {width} {height}] /Contents 4 0 R "
        "/Resources << /Font << /F1 5 0 R /F2 6 0 R >> >> >>".encode(),
        b"<< /Length "
        + str(len(stream)).encode()
        + b" >>\nstream\n"
        + stream
        + b"\nendstream",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>",
        b"<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica-Bold >>",
        (
            b"<< /Title (Write-Through Caching for Job Queues) /Author (A. Example) /CreationDate (D:20260101000000Z) >>"
        ),
    ]
    out = io.BytesIO()
    out.write(b"%PDF-1.4\n")
    offsets = []
    for i, body in enumerate(objs, 1):
        offsets.append(out.tell())
        out.write(f"{i} 0 obj\n".encode() + body + b"\nendobj\n")
    xref = out.tell()
    out.write(f"xref\n0 {len(objs) + 1}\n0000000000 65535 f \n".encode())
    for off in offsets:
        out.write(f"{off:010d} 00000 n \n".encode())
    out.write(
        f"trailer\n<< /Size {len(objs) + 1} /Root 1 0 R /Info 7 0 R >>\nstartxref\n{xref}\n%%EOF\n".encode()
    )
    return out.getvalue()


def zip_bytes(files: dict[str, str]) -> bytes:
    out = io.BytesIO()
    with zipfile.ZipFile(out, "w", zipfile.ZIP_DEFLATED) as z:
        for name, text in files.items():
            info = zipfile.ZipInfo(name, date_time=(2026, 1, 1, 0, 0, 0))
            info.compress_type = zipfile.ZIP_DEFLATED
            z.writestr(info, text)
    return out.getvalue()


W = 'xmlns:w="http://schemas.openxmlformats.org/wordprocessingml/2006/main"'


def docx(paras: list[tuple[str | None, str]]) -> bytes:
    body = "".join(
        f'<w:p>{f'<w:pPr><w:pStyle w:val="{style}"/></w:pPr>' if style else ""}<w:r><w:t xml:space="preserve">{text}</w:t></w:r></w:p>'
        for style, text in paras
    )
    return zip_bytes(
        {
            "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/word/document.xml" ContentType="application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml"/>'
            "</Types>",
            "_rels/.rels": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="word/document.xml"/>'
            "</Relationships>",
            "word/document.xml": f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><w:document {W}><w:body>{body}</w:body></w:document>',
        }
    )


def xlsx(rows: list[list[str]]) -> bytes:
    def cell(r: int, c: int, v: str) -> str:
        ref = f"{chr(65 + c)}{r}"
        if v.replace(".", "", 1).isdigit():
            return f'<c r="{ref}"><v>{v}</v></c>'
        return f'<c r="{ref}" t="inlineStr"><is><t>{v}</t></is></c>'

    sheet = "".join(
        f'<row r="{r}">' + "".join(cell(r, c, v) for c, v in enumerate(row)) + "</row>"
        for r, row in enumerate(rows, 1)
    )
    ns = 'xmlns="http://schemas.openxmlformats.org/spreadsheetml/2006/main"'
    rel = (
        'xmlns:r="http://schemas.openxmlformats.org/officeDocument/2006/relationships"'
    )
    return zip_bytes(
        {
            "[Content_Types].xml": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Types xmlns="http://schemas.openxmlformats.org/package/2006/content-types">'
            '<Default Extension="rels" ContentType="application/vnd.openxmlformats-package.relationships+xml"/>'
            '<Default Extension="xml" ContentType="application/xml"/>'
            '<Override PartName="/xl/workbook.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml"/>'
            '<Override PartName="/xl/worksheets/sheet1.xml" ContentType="application/vnd.openxmlformats-officedocument.spreadsheetml.worksheet+xml"/>'
            "</Types>",
            "_rels/.rels": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument" Target="xl/workbook.xml"/>'
            "</Relationships>",
            "xl/workbook.xml": f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><workbook {ns} {rel}>'
            '<sheets><sheet name="Capacity" sheetId="1" r:id="rId1"/></sheets></workbook>',
            "xl/_rels/workbook.xml.rels": '<?xml version="1.0" encoding="UTF-8" standalone="yes"?>'
            '<Relationships xmlns="http://schemas.openxmlformats.org/package/2006/relationships">'
            '<Relationship Id="rId1" Type="http://schemas.openxmlformats.org/officeDocument/2006/relationships/worksheet" Target="worksheets/sheet1.xml"/>'
            "</Relationships>",
            "xl/worksheets/sheet1.xml": f'<?xml version="1.0" encoding="UTF-8" standalone="yes"?><worksheet {ns}><sheetData>{sheet}</sheetData></worksheet>',
        }
    )


def wav() -> bytes:
    out = io.BytesIO()
    with wave.open(out, "wb") as w:
        w.setnchannels(1)
        w.setsampwidth(2)
        w.setframerate(8000)
        w.writeframes(
            b"".join(
                struct.pack("<h", int(3000 * math.sin(2 * math.pi * 440 * i / 8000)))
                for i in range(4000)
            )
        )
    return out.getvalue()


def main() -> None:
    p = argparse.ArgumentParser(
        description=__doc__, formatter_class=argparse.RawDescriptionHelpFormatter
    )
    p.add_argument("dest", type=Path)
    p.add_argument("--no-png", action="store_true")
    p.add_argument(
        "--wav", action="store_true", help="also write media/beep.wav (media probe)"
    )
    a = p.parse_args()
    d = a.dest
    (d / "papers").mkdir(parents=True, exist_ok=True)
    (d / "papers/write_through.pdf").write_bytes(pdf(PAPER))
    (d / "office").mkdir(exist_ok=True)
    (d / "office/runbook.docx").write_bytes(
        docx(
            [
                ("Heading1", "Queue Runbook"),
                (
                    None,
                    "When the queue backs up, call run_job for each pending job and watch the cache hit rate.",
                ),
                ("Heading2", "Escalation"),
                (
                    None,
                    "Page the on-call owner of JobQueue if latency exceeds 2 seconds.",
                ),
            ]
        )
    )
    (d / "office/capacity.xlsx").write_bytes(
        xlsx([["service", "jobs_per_s"], ["JobQueue", "120"], ["Cache", "900"]])
    )
    if a.wav:
        (d / "media").mkdir(exist_ok=True)
        (d / "media/beep.wav").write_bytes(wav())
    if not a.no_png:
        (d / "images").mkdir(exist_ok=True)
        with tempfile.TemporaryDirectory() as t:
            src = Path(t, "diagram.pdf")
            src.write_bytes(pdf(DIAGRAM, 560, 220, background=True))
            subprocess.run(
                [
                    "sips",
                    "-s",
                    "format",
                    "png",
                    "-s",
                    "formatOptions",
                    "default",
                    str(src),
                    "--out",
                    str(d / "images/architecture.png"),
                ],
                check=True,
                capture_output=True,
            )


if __name__ == "__main__":
    main()
