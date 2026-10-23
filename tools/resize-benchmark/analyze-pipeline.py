#!/usr/bin/env python3
"""Validate raw pipeline samples and run FFmpeg quality filters on decoded crops."""
import argparse
import hashlib
import html
import json
from pathlib import Path
import re
import statistics
import subprocess

parser = argparse.ArgumentParser()
parser.add_argument('directory', type=Path)
args = parser.parse_args()
directory = args.directory
results = json.loads((directory / 'results.json').read_text())
rows = results['rows']
expected = 2 * 2 * 3 * 4 * results['repeats']
assert len(rows) == expected, (len(rows), expected)
assert results['warmups'] == expected
for row in rows:
    n = results['frames_per_run']
    assert row['requested_frames'] == n
    assert row['decoded_frames'] == row['encoded_frames'] == n - row['skipped_frames']
    assert len(row['packet_bytes']) == n
    assert sum(row['packet_bytes']) == row['payload_bytes']
    assert sum(b == 0 for b in row['packet_bytes']) == row['skipped_frames']
    if 'displayed_input_age_frames' in row:
        assert len(row['displayed_input_age_frames']) == n
        last_index = 0
        for i, b in enumerate(row['packet_bytes']):
            if b:
                last_index = i
            assert row['displayed_input_age_frames'][i] == i - last_index
        assert last_index == row['last_decoded_input_index']
    for stage in ['scale', 'convert', 'encode', 'pipeline']:
        assert len(row[stage]['samples_ms']) == n
    for i, total in enumerate(row['pipeline']['samples_ms']):
        assert abs(total - sum(row[s]['samples_ms'][i] for s in ['scale', 'convert', 'encode'])) < 1e-7


def compare(reference, decoded):
    graph = '[0:v]format=yuv444p,split[a][b];[1:v]format=yuv444p,split[c][d];[a][c]ssim[s];[b][d]psnr[p]'
    command = ['ffmpeg', '-hide_banner', '-nostdin', '-i', str(reference), '-i', str(decoded),
               '-filter_complex', graph, '-map', '[s]', '-map', '[p]', '-f', 'null', '-']
    p = subprocess.run(command, capture_output=True, text=True, timeout=30, check=True)
    ssim = re.search(r'SSIM Y:.*?All:([0-9.]+)', p.stderr)
    psnr = re.search(r'PSNR y:.*?average:([0-9.inf]+)', p.stderr)
    if not ssim or not psnr:
        raise RuntimeError(p.stderr)
    return {'ssim_yuv444_all': float(ssim[1]), 'psnr_yuv444_db': None if psnr[1] == 'inf' else float(psnr[1])}


quality = []
for row in rows:
    if not row['artifacts']:
        continue
    path = Path(row['artifacts'])
    result = {k: row[k] for k in ['source', 'target', 'scene', 'profile', 'method', 'last_decoded_input_index']}
    result['compression_only'] = compare(path / 'compression-reference.png', path / 'compression-decoded.png')
    result['display_at_native_size'] = compare(path / 'desktop-reference.png', path / 'desktop-displayed.png')
    result['files'] = {p.name: hashlib.sha256(p.read_bytes()).hexdigest() for p in sorted(path.glob('*.png'))}
    quality.append(result)
    print('quality', path.name, result['display_at_native_size'], flush=True)
results['quality'] = quality
results['quality_method'] = 'FFmpeg format=yuv444p -> PSNR / SSIM, on 640x220 native text ROI; compression-only ROI at encoder resolution; first measured repeat only. Display reconstruction uses image Triangle, not actual GPU/browser rendering.'
results['ffmpeg'] = subprocess.check_output(['ffmpeg', '-version'], text=True).splitlines()[0]
(directory / 'results.json').write_text(json.dumps(results, indent=2) + '\n')

# Self-contained local comparison: all PNGs are adjacent, no external services.
parts = ['<!doctype html><meta charset="utf-8"><title>Encoder scaling comparison</title>',
         '<style>body{font:14px system-ui;margin:24px;color:#202735;background:#edf0f4}section{background:white;padding:20px;margin:24px 0}figure{margin:16px 0}img{display:block;max-width:none;border:1px solid #cad1da}small{color:#53657a}</style>',
         '<h1>Decoded text at native display size</h1>',
         '<p>Each image is a 640 × 220 crop shown at 1:1. The desktop remains at its source resolution. Downscaled streams are enlarged with a common bilinear display approximation. Static is the last repeated input; scrolling includes any stale final output.</p>']
for source in [1920, 3840]:
    for scene in ['static', 'scroll']:
        for profile in ['jpeg_q75', 'h264_default', 'h264_3mbps']:
            parts.append(f'<section><h2>{source}px · {scene} · {profile}</h2>')
            selection = [r for r in rows if r['source'][0] == source and r['scene'] == scene and r['profile'] == profile and r['repeat'] == 0]
            first = Path(selection[0]['artifacts']).relative_to(directory)
            parts.append(f'<figure><figcaption>Original lossless desktop</figcaption><img src="{first}/desktop-reference.png"></figure>')
            for row in selection:
                artifact = Path(row['artifacts']).relative_to(directory)
                q = next(q for q in quality if all(q[k] == row[k] for k in ['source', 'scene', 'profile', 'method']))
                parts.append(f'<figure><figcaption>{html.escape(row["method"])} · target {row["target"]} · skip {row["skipped_frames"]} · SSIM {q["display_at_native_size"]["ssim_yuv444_all"]:.4f}</figcaption><img src="{artifact}/desktop-displayed.png"></figure>')
            parts.append('</section>')
(directory / 'quality.html').write_text('\n'.join(parts) + '\n')

aggregates = []
for source in [1920, 3840]:
    for scene in ['static', 'scroll']:
        for profile in ['jpeg_q75', 'h264_default', 'h264_3mbps']:
            selection = [r for r in rows if r['source'][0] == source and r['scene'] == scene and r['profile'] == profile]
            base = statistics.median(r['payload_bytes'] for r in selection if r['method'] == 'native')
            for method in ['native', 'image_triangle', 'fir_bilinear', 'fir_lanczos3']:
                group = [r for r in selection if r['method'] == method]
                aggregates.append({'source_width': source, 'scene': scene, 'profile': profile, 'method': method,
                    **{stage + '_p50_ms': statistics.median(r[stage]['p50_ms'] for r in group) for stage in ['scale', 'convert', 'encode', 'pipeline']},
                    'pipeline_p95_ms': statistics.median(r['pipeline']['p95_ms'] for r in group),
                    'payload_bytes': statistics.median(r['payload_bytes'] for r in group),
                    'payload_reduction_pct': (1-statistics.median(r['payload_bytes'] for r in group)/base)*100,
                    'skipped_frames': [r['skipped_frames'] for r in group]})
(directory / 'aggregates.json').write_text(json.dumps(aggregates, indent=2) + '\n')
print(json.dumps({'validated_runs': len(rows), 'quality_cases': len(quality), 'comparison': str(directory / 'quality.html')}))
