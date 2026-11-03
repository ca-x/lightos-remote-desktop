#!/usr/bin/env python3
"""Build the VideoToolbox worker and embed its private LGPL FFmpeg libraries."""
import argparse
import hashlib
import json
import os
from pathlib import Path
import shutil
import subprocess
from runtime_codec import sdk, notices

parser = argparse.ArgumentParser()
parser.add_argument('--source', type=Path, required=True)
parser.add_argument('--output', type=Path, required=True)
parser.add_argument('--target', required=True)
parser.add_argument('--identity', required=True)
a = parser.parse_args()
source, output = a.source.resolve(), a.output.resolve()
cache = source.parent / 'ffmpeg-cache'
ffmpeg_source = sdk('macos-source', cache)
arch = 'arm64' if a.target.startswith('aarch64') else 'x86_64'
minimum = '13.0'
root = cache / ('videotoolbox-' + arch)
prefix = root / 'install'
patched_source = root / 'source'
patch = source / 'scripts/ffmpeg-videotoolbox-low-delay.patch'
sdkroot = subprocess.check_output(['xcrun', '--sdk', 'macosx', '--show-sdk-path'], text=True).strip()
configuration = [str(patched_source / 'configure'), '--prefix=' + str(prefix), '--arch=' + arch,
    '--target-os=darwin', '--cc=clang -arch ' + arch, '--enable-cross-compile', '--sysroot=' + sdkroot,
    '--extra-cflags=-mmacosx-version-min=' + minimum, '--extra-ldflags=-mmacosx-version-min=' + minimum,
    '--disable-autodetect', '--disable-programs', '--disable-doc', '--disable-network', '--disable-debug',
    '--disable-static', '--enable-shared', '--enable-pic', '--disable-everything', '--disable-avdevice',
    '--disable-avfilter', '--disable-swresample', '--enable-decoder=h264', '--enable-parser=h264',
    '--enable-bsf=h264_mp4toannexb', '--enable-encoder=h264_videotoolbox', '--enable-videotoolbox']
fingerprint = hashlib.sha256(json.dumps(configuration).encode() + patch.read_bytes() + Path(__file__).read_bytes()).hexdigest()
marker = prefix / '.configuration'
if not marker.exists() or marker.read_text() != fingerprint:
    if root.exists(): shutil.rmtree(root)
    root.mkdir(parents=True, exist_ok=True)
    shutil.copytree(ffmpeg_source, patched_source)
    subprocess.run(['patch', '-p1', '-i', str(patch)], cwd=patched_source, check=True)
    with (root / 'build.log').open('w') as log:
        subprocess.run(configuration, cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['make', '-j4'], cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
        subprocess.run(['make', 'install'], cwd=root, stdout=log, stderr=subprocess.STDOUT, check=True)
    marker.write_text(fingerprint)
env = dict(os.environ)
env.update(FFMPEG_DIR=str(prefix), MACOSX_DEPLOYMENT_TARGET=minimum,
    RUSTFLAGS='-C link-arg=-Wl,-rpath,@executable_path/../Frameworks')
subprocess.run(['cargo', 'build', '--release', '--locked', '--target', a.target, '-p', 'media-codec-ffmpeg',
                '--features', 'native,media-codec/bundled-jpeg'], cwd=source, env=env, check=True)
contents = output / 'Lazycat Remote.app/Contents'
worker = contents / 'MacOS/lightos-codec-ffmpeg'
shutil.copy2(Path(env['CARGO_TARGET_DIR']) / a.target / 'release/lightos-codec-ffmpeg', worker)
worker.chmod(0o755)
frameworks = contents / 'Frameworks'
frameworks.mkdir(parents=True, exist_ok=True)
libraries = []
for name in ['avcodec', 'avformat', 'avutil', 'swscale']:
    candidates = [p for p in (prefix / 'lib').glob('lib' + name + '.*.dylib') if len(p.name.split('.')) == 3]
    if len(candidates) != 1: raise RuntimeError('Missing FFmpeg library: ' + name)
    destination = frameworks / candidates[0].name
    shutil.copy2(candidates[0], destination)
    libraries.append(destination)
for binary in [*libraries, worker]:
    dependencies = subprocess.check_output(['otool', '-L', str(binary)], text=True).splitlines()[1:]
    for line in dependencies:
        dependency = line.strip().split(' (', 1)[0]
        if dependency.startswith(str(prefix) + '/'):
            subprocess.run(['install_name_tool', '-change', dependency, '@rpath/' + Path(dependency).name, str(binary)], check=True)
    if binary in libraries:
        subprocess.run(['install_name_tool', '-id', '@rpath/' + binary.name, str(binary)], check=True)
    subprocess.run(['codesign', '--force', '--sign', a.identity, '--options', 'runtime', '--timestamp=none', str(binary)], check=True)
notices('macos-source', ffmpeg_source, output, ' '.join(configuration))

shutil.copy2(patch, output / 'FFMPEG-CHANGES.patch')
