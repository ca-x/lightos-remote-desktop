#!/usr/bin/env node
// Render synthetic pixels only, using the already available agent-browser.
import {execFileSync} from 'node:child_process';
import {readFileSync,writeFileSync,mkdirSync} from 'node:fs';
import {resolve} from 'node:path';
import {createHash} from 'node:crypto';
import {pathToFileURL} from 'node:url';
const directory=resolve(process.argv[2]??'.local/encoder-scaling/fixtures');
const agent=process.argv[3]??'agent-browser';
const run=(...args)=>JSON.parse(execFileSync(agent,['--session','lrd-encoder-scaling-fixtures',...args,'--json'],{encoding:'utf8'}));
mkdirSync(directory,{recursive:true});
const rows=[];
try {
  run('open',pathToFileURL(resolve('tools/resize-benchmark/fixtures/desktop.html')).href);
  for(const [width,height] of [[1920,1080],[3840,2160]]) {
    run('set','viewport',String(width),String(height));
    run('eval','document.querySelector("#dimensions").textContent=innerWidth+" × "+innerHeight; document.fonts.ready.then(()=>true)');
    const file=`desktop-${width}x${height}.png`;
    run('screenshot',resolve(directory,file));
    rows.push({file,width,height,sha256:createHash('sha256').update(readFileSync(resolve(directory,file))).digest('hex')});
  }
  const environment=run('eval','({userAgent:navigator.userAgent,devicePixelRatio,fonts:[...document.fonts].map(f=>f.family)})').data.result;
  const fonts=execFileSync('fc-match',['Liberation Mono','-f','%{family}: %{file}\n'],{encoding:'utf8'}).trim();
  writeFileSync(resolve(directory,'manifest.json'),JSON.stringify({environment,fonts,rows},null,2)+'\n');
  console.log(JSON.stringify({directory,rows}));
} finally {run('close');}
