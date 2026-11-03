"""Pinned FFmpeg inputs for the self-contained LPK runtimes."""
import hashlib
import json
from pathlib import Path
import re
import shutil
import tarfile
import urllib.request
import zipfile

REPO = Path(__file__).resolve().parent.parent
LOCK = json.loads((REPO / 'config/codec-dependencies.json').read_text())
LIBRARIES = ('avcodec', 'avformat', 'avutil', 'swscale', 'swresample')

def sdk(target, cache=None):
    cache = Path(cache or REPO / '.local/codec-dependencies')
    cache.mkdir(parents=True, exist_ok=True)
    entry = LOCK[target]
    archive = cache / entry['url'].rsplit('/', 1)[1]
    if not archive.exists():
        temporary = archive.with_suffix(archive.suffix + '.download')
        with urllib.request.urlopen(entry['url'], timeout=60) as response, temporary.open('wb') as output:
            shutil.copyfileobj(response, output)
        temporary.replace(archive)
    if hashlib.sha256(archive.read_bytes()).hexdigest() != entry['sha256']:
        raise RuntimeError('FFmpeg download checksum mismatch: ' + target)
    directory = cache / entry['sha256'][:16]
    if not directory.exists():
        directory.mkdir()
        try:
            if archive.suffix == '.zip':
                with zipfile.ZipFile(archive) as package:
                    for name in package.namelist():
                        if Path(name).is_absolute() or '..' in Path(name).parts:
                            raise RuntimeError('Unsafe FFmpeg archive path')
                    package.extractall(directory)
            else:
                with tarfile.open(archive) as package:
                    # macOS's system Python can predate tarfile's data filter.
                    # Validate paths/links explicitly before extracting pinned SDKs.
                    base = directory.resolve()
                    for member in package.getmembers():
                        destination = (base / member.name).resolve()
                        if not destination.is_relative_to(base) or member.isdev():
                            raise RuntimeError('Unsafe FFmpeg archive entry')
                        if member.issym() or member.islnk():
                            link = (destination.parent if member.issym() else base) / member.linkname
                            if not link.resolve().is_relative_to(base):
                                raise RuntimeError('Unsafe FFmpeg archive link')
                        member.mode &= 0o777
                    package.extractall(directory)
        except BaseException:
            shutil.rmtree(directory)
            raise
    roots = list(directory.iterdir())
    if len(roots) != 1 or not roots[0].is_dir():
        raise RuntimeError('Unexpected FFmpeg archive layout')
    return roots[0]

def copy_libraries(target, source, destination):
    destination.mkdir(parents=True, exist_ok=True)
    for library in LIBRARIES:
        pattern = library + '-*.dll' if target == 'win32-x64' else 'lib' + library + '.so.*'
        candidates = list((source / ('bin' if target == 'win32-x64' else 'lib')).glob(pattern))
        if target != 'win32-x64':
            candidates = [p for p in candidates if re.search(r'\.so\.\d+$', p.name)]
        if len(candidates) != 1:
            raise RuntimeError('Missing or ambiguous FFmpeg library: ' + library)
        shutil.copy2(candidates[0], destination / candidates[0].name)

def notices(target, source, destination, configuration=''):
    notice = {'dependency': 'FFmpeg', 'license': 'LGPL-2.1-or-later', **LOCK[target], 'configuration': configuration,
              'build_project': 'https://github.com/BtbN/FFmpeg-Builds/tree/autobuild-2026-11-02-12-51' if target != 'macos-source' else 'https://ffmpeg.org/'}
    (destination / 'NOTICE.ffmpeg.json').write_text(json.dumps(notice, indent=2) + '\n')
    for name in ['LICENSE.txt', 'COPYING.LGPLv2.1']:
        if (source / name).exists():
            shutil.copy2(source / name, destination / 'LICENSE.ffmpeg.txt')
            break
    else:
        raise RuntimeError('FFmpeg license is missing from the pinned input')
