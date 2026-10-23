import { encodeBinaryCommand, framesProtocol, protobufProtocol, videoHeader } from "../web/src/wire";

type RunOptions = { mode: "legacy" | "frames" | "protobuf"; codec: "jpeg" | "h264"; address: string };
const percentile = (values: number[], fraction: number) => [...values].sort((a,b)=>a-b)[Math.max(0,Math.ceil(values.length*fraction)-1)] ?? 0;

async function runWireBenchmark(options: RunOptions) {
  const url = new URL(`ws://${options.address}/__wire_bench/ws`);
  url.searchParams.set("codec",options.codec);
  const socket = new WebSocket(url,options.mode === "protobuf" ? [protobufProtocol] : options.mode === "frames" ? [framesProtocol] : []);
  socket.binaryType = "arraybuffer";
  const canvas = document.querySelector("canvas")!;
  const drawing = canvas.getContext("2d")!;
  const decodeMs: number[] = [];
  const parseMs: number[] = [];
  const pending = new Map<number,{id:number;started:number}>();
  let frameId: number | null = null;
  let decoder: VideoDecoder | null = null;
  let error: Error | null = null;
  let first = 0, last = 0, received = 0, drawn = 0, videoBytes = 0, upstreamBytes = 0, downstreamBytes = 0;
  let upstreamMessages = 0, downstreamMessages = 0;
  let expected = 0;
  let started = 0;
  const completed = new Promise<Record<string,unknown>>((resolve,reject) => {
    const watchdog = setTimeout(()=>reject(new Error("benchmark timed out")),30000);
    const fail = (cause: unknown) => { error = cause instanceof Error ? cause : new Error(String(cause)); clearTimeout(watchdog); socket.close(); reject(error); };
    const send = (command: Record<string,unknown>) => {
      const binary = socket.protocol === protobufProtocol ? encodeBinaryCommand(command) : null;
      const data = binary ?? JSON.stringify(command);
      upstreamBytes += typeof data === "string" ? new TextEncoder().encode(data).length : data.byteLength;
      upstreamMessages++;
      socket.send(data);
    };
    const rendered = (id:number,begin:number) => {
      const now = performance.now();
      first ||= now; last = now; drawn++;
      decodeMs.push(now-begin);
      // Repeatable 60-Hz pointer stream plus one key press/release per second.
      for (const x of [(drawn % 97) / 97, (drawn % 89) / 89]) send({type:"move",x,y:0.5});
      if ((drawn-1)%30 === 0) { send({type:"key",code:30,pressed:true}); send({type:"key",code:30,pressed:false}); }
      send({type:"frame_ack",id});
    };
    socket.onmessage = async ({data}) => {
      try {
        const parseStarted = performance.now();
        downstreamMessages++;
        downstreamBytes += typeof data === "string" ? new TextEncoder().encode(data).length : data.byteLength;
        if (typeof data === "string") {
          const value = JSON.parse(data);
          parseMs.push(performance.now()-parseStarted);
          if (value.type === "bench_ready") {
            expected = value.count;
            if (options.codec === "h264") {
              const config = {codec:value.configuration,optimizeForLatency:true,hardwareAcceleration:"prefer-software" as const};
              const support = await VideoDecoder.isConfigSupported(config);
              if (!support.supported) throw new Error(`browser does not support ${value.configuration}`);
              decoder = new VideoDecoder({output(frame) {
                try {
                  const entry = pending.get(frame.timestamp);
                  if (!entry) throw new Error("unmatched H264 timestamp");
                  pending.delete(frame.timestamp);
                  drawing.drawImage(frame,0,0);
                  rendered(entry.id,entry.started);
                } catch (cause) { fail(cause); } finally { frame.close(); }
              },error:fail});
              decoder.configure(config);
            }
            started=performance.now();send({type:"bench_start"});
          } else if (value.type === "frame_id") frameId = value.id;
          else if (value.type === "bench_done") {
            clearTimeout(watchdog);
            if (error || drawn !== expected || received !== expected || pending.size) throw error ?? new Error(`incomplete draw ${drawn}/${expected}`);
            resolve({mode:options.mode,codec:options.codec,protocol:socket.protocol,frames:drawn,elapsedMs:performance.now()-started,
              fps:(drawn-1)*1000/(last-first),videoBytes,upstreamBytes,downstreamBytes,upstreamMessages,downstreamMessages,
              decodeDrawP50Ms:percentile(decodeMs,.5),decodeDrawP95Ms:percentile(decodeMs,.95),
              parseP50Ms:percentile(parseMs,.5),parseP95Ms:percentile(parseMs,.95),server:value});
            socket.close();
          }
          return;
        }
        const header = videoHeader(data,socket.protocol !== "");
        if (!header) throw new Error("invalid video packet");
        const id = header.id ?? frameId;frameId=null;
        if (id !== 1000+received) throw new Error(`frame ID sequence broken: ${id}`);
        received++;videoBytes+=data.byteLength-header.offset;
        parseMs.push(performance.now()-parseStarted);
        const begin = performance.now();
        if (header.kind === "jpeg") {
          // Same copy and decoder operation as createClient in every mode.
          const bitmap = await createImageBitmap(new Blob([data.slice(header.offset)],{type:"image/jpeg"}));
          try { drawing.drawImage(bitmap,0,0);rendered(id!,begin); } finally { bitmap.close(); }
        } else {
          pending.set(header.timestamp,{id:id!,started:begin});
          decoder!.decode(new EncodedVideoChunk({type:header.key?"key":"delta",timestamp:header.timestamp,data:new Uint8Array(data,header.offset)}));
        }
      } catch (cause) { fail(cause); }
    };
    socket.onerror = ()=>fail(new Error("WebSocket error"));
    socket.onclose = ()=> { if (drawn !== expected || drawn === 0) fail(new Error("WebSocket closed before completion")); };
  });
  try { return await completed; } finally { if (decoder && decoder.state !== "closed") decoder.close();socket.close(); }
}
Object.assign(window,{runWireBenchmark});
