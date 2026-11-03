#!/usr/bin/env python3
"""Build the viewing-device input helper as part of the remote desktop runtime."""
import argparse
import os
from pathlib import Path
import re
import subprocess
import sys

p=argparse.ArgumentParser();p.add_argument('--target',required=True);p.add_argument('--output',type=Path,required=True);p.add_argument('--identity',default='-');a=p.parse_args()
root=Path(__file__).resolve().parent.parent;source=root/'native/viewer-input';output=a.output.resolve();output.parent.mkdir(parents=True,exist_ok=True)
def run(args):subprocess.run(args,check=True)
if a.target.startswith('linux'):
    build=output.parent/'.viewer-input-build';build.mkdir(exist_ok=True);files=[]
    for name in ['keyboard-shortcuts-inhibit-unstable-v1','wlr-layer-shell-unstable-v1','wlr-foreign-toplevel-management-unstable-v1','xdg-shell']:
        xml=Path('/usr/share/wayland-protocols/stable/xdg-shell/xdg-shell.xml') if name=='xdg-shell' else source/'protocols'/(name+'.xml')
        run(['wayland-scanner','client-header',str(xml),str(build/(name+'-client-protocol.h'))])
        code=build/(name+'-protocol.c');run(['wayland-scanner','private-code',str(xml),str(code)]);files.append(str(code))
    flags=subprocess.check_output(['pkg-config','--cflags','--libs','wayland-client','x11'],text=True).split()
    run(['cc','-O2','-Wall','-Wextra','-Wno-unused-parameter','-Wno-misleading-indentation','-I'+str(build),str(source/'linux.c'),str(source/'wayland.c'),*files,*flags,'-o',str(output)])
elif a.target.startswith('darwin'):
    run(['clang','-O2','-arch','arm64' if a.target.endswith('arm64') else 'x86_64','-mmacosx-version-min=13.0','-fobjc-arc',str(source/'macos.m'),'-framework','AppKit','-framework','ApplicationServices','-o',str(output)])
    run(['codesign','--force','--sign',a.identity,'--identifier','org.lightos.remote-desktop.input-capture','--options','runtime','--timestamp=none',str(output)])
elif a.target=='win32-x64':
    if sys.platform=='win32':run(['cl','/nologo','/O2','/MT','/EHsc',str(source/'windows.cpp'),'/Fe:'+str(output),'/Fo:'+str(output.with_suffix('.obj')),'/link','user32.lib'])
    else:
        sdk=Path(os.environ.get('XWIN_ROOT',Path.home()/'.cache/cargo-xwin/xwin'))
        run(['clang-cl','--target=x86_64-pc-windows-msvc','-fuse-ld=lld','/nologo','/O2','/MT','/EHsc',*['/imsvc'+str(sdk/q) for q in ['crt/include','sdk/include/ucrt','sdk/include/shared','sdk/include/um']],str(source/'windows.cpp'),'/Fe:'+str(output),'/Fo:'+str(output.with_suffix('.obj')),'/link',*['/libpath:'+str(sdk/q) for q in ['crt/lib/x86_64','sdk/lib/um/x86_64','sdk/lib/ucrt/x86_64']],'user32.lib'])
    output.with_suffix('.obj').unlink(missing_ok=True)
else:raise SystemExit('Unsupported viewer input target')
notices=['Viewing-device keyboard capture. Wayland protocol copyright notices:\n']
for xml in sorted((source/'protocols').glob('*.xml')):
    text=xml.read_text();notice=re.search(r'<copyright>(.*?)</copyright>',text,re.S)
    if notice:notices.append(xml.name+'\n'+notice.group(1).strip())
(output.parent/'NOTICE.input.txt').write_text('\n\n'.join(notices)+'\n')
