#!/usr/bin/env node
// Deterministic replay over real Rust -> TCP -> Chromium -> canvas -> render ACK.
import { spawn, execFileSync } from 'node:child_process';
import { createServer, connect } from 'node:net';
import { readFileSync, writeFileSync, mkdirSync, existsSync, openSync } from 'node:fs';
import { resolve } from 'node:path';
import { createHash } from 'node:crypto';
import { build } from 'vite';

class CDP {
  constructor(socket) {
    this.socket=socket;this.id=0;this.pending=new Map();this.listeners=new Set();
    socket.addEventListener('message',({data})=>{
      const value=JSON.parse(data);
      if(value.id) {const p=this.pending.get(value.id);this.pending.delete(value.id);if(value.error)p?.reject(new Error(JSON.stringify(value.error)));else p?.resolve(value.result);}
      else for(const listener of this.listeners)listener(value.method,value.params??{},value.sessionId);
    });
  }
  static async open(url) {const socket=new WebSocket(url);await new Promise((resolve,reject)=>{socket.addEventListener('open',resolve,{once:true});socket.addEventListener('error',reject,{once:true});});return new CDP(socket);}
  call(method,params={},sessionId) {const id=++this.id;return new Promise((resolve,reject)=>{this.pending.set(id,{resolve,reject});this.socket.send(JSON.stringify({id,method,params,...(sessionId?{sessionId}:{})}));});}
  onEvent(callback) {this.listeners.add(callback);return()=>this.listeners.delete(callback);}
  close(){this.socket.close();}
}

const options = new Map(process.argv.slice(2).reduce((pairs,value,index,args)=>value.startsWith('--')?[...pairs,[value,args[index+1]]]:pairs,[]));
const directory = resolve(options.get('--directory') ?? '.local/wire-bench');
const repeats = Number(options.get('--repeats') ?? 5);
const agent = options.get('--agent-browser') ?? 'agent-browser';
if (!Number.isInteger(repeats) || repeats < 1 || repeats > 20) throw new Error('repeats must be 1..20');
mkdirSync(directory,{recursive:true});
execFileSync('python3',['scripts/prepare-wire-benchmark.py',directory,'--frames',options.get('--frames') ?? '120'],{stdio:'inherit'});
await build({configFile:false,build:{outDir:directory,emptyOutDir:false,lib:{entry:'scripts/benchmark-wire-browser.ts',name:'WireBenchmark',formats:['iife'],fileName:()=> 'browser.iife.js'}}});
const serverLog = openSync(`${directory}/server.log`,'w');
if (existsSync(`${directory}/address.json`)) writeFileSync(`${directory}/address.json`,'');
const server = spawn('cargo',['test','--release','--locked','--lib','--no-default-features','browser_wire_benchmark_server','--','--ignored','--nocapture'],{
  env:{...process.env,LIGHTOS_WIRE_BENCH_DIR:directory},stdio:['ignore',serverLog,serverLog],detached:true,
});
const wait = ms=>new Promise(resolve=>setTimeout(resolve,ms));
let browser, proxy;
process.once('SIGINT',()=>{try {process.kill(-server.pid,'SIGINT');}catch{}if(proxy)proxy.close();if(browser)browser.close();process.exitCode=130;});
try {
  let address, nodelayAddress;
  for (let i=0;i<900;i++) {
    if (server.exitCode !== null) throw new Error(`server exited: see ${directory}/server.log`);
    try { ({address,nodelayAddress}=JSON.parse(readFileSync(`${directory}/address.json`,'utf8')));if(address)break; } catch {}
    await wait(500);
  }
  if (!address) throw new Error('server startup timed out');
  const session = 'lrd-binary-experiment';
  const runAgent = (...args)=>JSON.parse(execFileSync(agent,['--session',session,...args,'--json'],{encoding:'utf8'}));
  runAgent('open','about:blank');
  const cdpUrl = runAgent('get','cdp-url').data.cdpUrl;
  browser = await CDP.open(cdpUrl);
  const targets = (await browser.call('Target.getTargets')).targetInfos.filter(t=>t.type==='page');
  const target = targets.find(t=>t.url==='about:blank') ?? targets[0];
  const {sessionId} = await browser.call('Target.attachToTarget',{targetId:target.targetId,flatten:true});
  const call = (method,params={})=>browser.call(method,params,sessionId);
  await call('Runtime.enable');await call('Page.enable');await call('Performance.enable');
  const evaluate = async expression => {
    const r = await call('Runtime.evaluate',{expression,returnByValue:true,awaitPromise:true});
    if(r.exceptionDetails)throw new Error(r.exceptionDetails.exception?.description ?? r.exceptionDetails.text);
    return r.result.value;
  };
  const navigate = async url=>{
    await call('Page.navigate',{url});
    for(let i=0;i<100;i++) {
      try { if(await evaluate('document.readyState === "complete"'))return; } catch {}
      await wait(50);
    }
    throw new Error(`navigation timeout: ${url}`);
  };
  const versions = await browser.call('Browser.getVersion');
  const samples = [], warmups = [];
  const processCpu = async()=>new Map((await browser.call('SystemInfo.getProcessInfo')).processInfo.map(p=>[p.id,p.cpuTime]));
  const metrics = async()=>Object.fromEntries((await call('Performance.getMetrics')).metrics.map(m=>[m.name,m.value]));
  const modes=['legacy','frames','protobuf'];
  for(const link of [{name:'loopback-production',delayMs:0,address:nodelayAddress},{name:'loopback-nagle-control',delayMs:0,address},{name:'40ms-rtt-production',delayMs:20,address:nodelayAddress}]) {
    proxy=await meteredProxy(link.address,link.delayMs);
    const proxyAddress=`127.0.0.1:${proxy.port}`;
    await navigate(`http://${proxyAddress}/__wire_bench`);
    for(let i=0;i<100;i++) { if(await evaluate('typeof window.runWireBenchmark === "function"'))break;await wait(50); }
    for(let repeat=-1;repeat<repeats;repeat++) {
      for(const codec of ['jpeg','h264']) {
        for(let i=0;i<modes.length;i++) {
          const mode=modes[(i+Math.max(0,repeat))%modes.length];
          const beforeCpu = await processCpu(), beforeMetrics = await metrics();
          const beforeConnections = proxy.connections.length;
          const data = await evaluate(`window.runWireBenchmark(${JSON.stringify({mode,codec,address:proxyAddress})})`);
          const afterMetrics = await metrics(), afterCpu = await processCpu();
          // Let the close handshake finish outside the measured interval.
          await wait(30);
          const wires = proxy.connections.slice(beforeConnections).filter(c=>c.websocket);
          if(wires.length!==1)throw new Error(`expected exactly one measured WebSocket; got ${wires.length}`);
          const wire = wires[0];
          let browserCpuMs=0;
          for(const [pid,seconds] of afterCpu) if(beforeCpu.has(pid))browserCpuMs+=(seconds-beforeCpu.get(pid))*1000;
          Object.assign(data,{repeat,link:link.name,wire:{upBytes:wire.upBytes,downBytes:wire.downBytes,upHandshakeBytes:wire.upHeaderBytes,downHandshakeBytes:wire.downHeaderBytes},
            browserCpuMs,browserScriptMs:(afterMetrics.ScriptDuration-beforeMetrics.ScriptDuration)*1000,
            browserTaskMs:(afterMetrics.TaskDuration-beforeMetrics.TaskDuration)*1000});
          (repeat<0?warmups:samples).push(data);
          writeFileSync(`${directory}/results.partial.json`,JSON.stringify({versions,warmups,samples},null,2));
          console.log(`${link.name} ${codec} ${mode} ${repeat<0?'warmup':repeat+1+'/'+repeats}: ${data.fps.toFixed(2)} fps, wire=${wire.downBytes}+${wire.upBytes} B, serverCPU=${data.server.server_cpu_ms.toFixed(1)} ms`);
        }
      }
    }
    proxy.close();proxy=null;
  }
  // Exercise the complete application client, decoder, ACK negotiation and HTTP route.
  await call('Network.enable');
  await call('Network.setExtraHTTPHeaders',{headers:{Authorization:`Basic ${Buffer.from('lightos:wire-benchmark-only').toString('base64')}`}});
  const smoke=[];
  for(const mode of modes) {
    const observed=[];
    const unsubscribe=browser.onEvent((method,params)=>{
      if(method==='Network.webSocketFrameReceived'||method==='Network.webSocketFrameSent') {
        const frame=params.response;
        if(frame.opcode===1||frame.opcode===2)observed.push({direction:method.endsWith('Sent')?'up':'down',opcode:frame.opcode,
          kind:frame.opcode===2?Buffer.from(frame.payloadData,'base64')[0]:JSON.parse(frame.payloadData).type});
      }
    });
    await navigate(`http://${nodelayAddress}/?wire=${mode}`);
    let state;
    for(let i=0;i<100;i++) {
      state=await evaluate('({visible:!document.querySelector("#desktop")?.hidden,width:document.querySelector("#desktop")?.width,version:document.querySelector("#software-version")?.textContent,state:document.querySelector("#state")?.textContent})');
      if(state.visible&&state.width===1280&&state.version?.startsWith('v')&&observed.some(f=>f.direction==='down'&&f.opcode===2)&&observed.some(f=>f.direction==='up'&&(f.kind==='frame_ack'||f.opcode===2)))break;
      await wait(100);
    }
    const video=observed.some(f=>f.direction==='down'&&f.opcode===2&&(mode==='legacy'?[1,3]:[5,6]).includes(f.kind));
    const ack=observed.some(f=>f.direction==='up'&&(mode==='protobuf'?f.opcode===2:f.kind==='frame_ack'));
    smoke.push({mode,...state,video,ack});unsubscribe();
    if(!video||!ack||!state.visible)throw new Error(`application smoke failed ${JSON.stringify(smoke.at(-1))}`);
    await navigate('about:blank');await wait(250);
  }
  const manifest=JSON.parse(readFileSync(`${directory}/manifest.json`,'utf8'));
  for(const codec of ['jpeg','h264']) {
    const expected=codec==='jpeg'?readFileSync(`${directory}/frame.jpg`).length*manifest.jpeg.files.length:readFileSync(`${directory}/stream.h264`).length;
    if(samples.some(s=>s.codec===codec&&s.videoBytes!==expected))throw new Error('compressed video payload changed across modes');
  }
  const result={
    source:execFileSync('git',['rev-parse','HEAD'],{encoding:'utf8'}).trim(),
    diffSha256:createHash('sha256').update(execFileSync('git',['diff','HEAD'])).digest('hex'),
    timestamp:new Date().toISOString(),versions,node:process.version,platform:process.platform,
    cpu:readFileSync('/proc/cpuinfo','utf8').match(/model name\s*:\s*(.*)/)?.[1],
    manifest,warmups,samples,smoke,
    limits:['Synthetic 1280x720 replay, real TCP/WebSocket and Chromium decoder/canvas. No physical desktop capture or Internet/TLS measurement.',
      'Fixed pre-encoded payloads isolate wire changes; no encoder/capture FPS claim.',
      'Draw durations end at canvas draw submission, not physical display presentation.',
      'Server CPU uses process getrusage; browser CPU sums existing Chrome processes. Main-thread ScriptDuration and TaskDuration come from CDP.',
      'TCP proxy counts actual WebSocket stream bytes after HTTP header boundaries, including close frames; no TCP/IP or TLS headers.',
      '40ms RTT is a user-space 20ms delay per direction, without simulated loss or a bandwidth cap. Proxy sockets explicitly use TCP_NODELAY; the production comparison enables TCP_NODELAY as src/main.rs already does; a separate no-NODELAY control explains small-write delays.',
      'Two normalized pointer moves per frame and one key press/release per 30 frames; one render ACK per frame; single frame in flight.',
      'Browser JPEG payload copy is held constant across modes; H264 passes a Uint8Array view.'],
  };
  writeFileSync(`${directory}/results.json`,JSON.stringify(result,null,2)+'\n');
  console.log(`Saved ${samples.length} measured samples and application smoke results to ${directory}/results.json`);
} finally {
  if(proxy)proxy.close();if(browser)browser.close();
  try {process.kill(-server.pid,'SIGINT');}catch{}
}

async function meteredProxy(address,delayMs) {
  const [host,port]=address.split(':');const connections=[],sockets=new Set(),timers=new Set();
  const server=createServer(client=>{
    const remote=connect({host,port:Number(port)});client.setNoDelay(true);remote.setNoDelay(true);sockets.add(client);sockets.add(remote);
    const record={upBytes:0,downBytes:0,upHeaderBytes:0,downHeaderBytes:0,websocket:false};connections.push(record);
    const meter=(direction)=>{
      let header=Buffer.alloc(0),ready=false;
      return chunk=>{
        if(ready)record[`${direction}Bytes`]+=chunk.length;
        else {
          header=Buffer.concat([header,chunk]);const end=header.indexOf('\r\n\r\n');
          if(end>=0) {record[`${direction}HeaderBytes`]=end+4;record[`${direction}Bytes`]+=header.length-end-4;ready=true;
            if(direction==='up')record.websocket=header.subarray(0,end).toString().toLowerCase().includes('upgrade: websocket');header=Buffer.alloc(0);}
        }
      };
    };
    const pipe=(source,destination,direction)=>{
      const count=meter(direction);
      source.on('data',chunk=>{count(chunk);if(delayMs===0)destination.write(chunk);else {
        const timer=setTimeout(()=>{timers.delete(timer);if(!destination.destroyed)destination.write(chunk);},delayMs);timers.add(timer);
      }});
      source.on('end',()=>{if(delayMs===0)destination.end();else {const timer=setTimeout(()=>{timers.delete(timer);destination.end();},delayMs+1);timers.add(timer);}});
      source.on('error',()=>destination.destroy());source.on('close',()=>sockets.delete(source));
    };
    pipe(client,remote,'up');pipe(remote,client,'down');
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  return {port:server.address().port,connections,close(){for(const timer of timers)clearTimeout(timer);for(const socket of sockets)socket.destroy();server.close();}};
}
