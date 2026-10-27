const MAX_IMAGE = 8 * 1024 * 1024;
const CHUNK = 48 * 1024;

interface ClipboardOptions {
  send: (command: Record<string, unknown>) => boolean;
  ready: () => boolean;
  session: () => number;
  images: () => boolean;
  release: () => void;
  text: (value: string, shift?: boolean) => void;
  message: (value: string) => void;
  shift?: () => boolean;
  signal?: AbortSignal;
}

export function createClipboard(options: ClipboardOptions) {
  let sequence = 0;
  let busy = false;
  let operation = 0;
  let pending: { id: number; step: number; resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> } | null = null;
  let socket: WebSocket | null = null, socketEpoch = 0, retry: ReturnType<typeof setTimeout> | null = null;
  let token: string | null = null, synchronize = false;
  let copyError:Error|null=null;
  let localActive=true;
  let mirrorEpoch=0, mirrorId:number|null=null, mirrorBusy=false;
  let mirroredKey:string|null=null;
  let outbox:Record<string,unknown>[]=[];
  type Copy = { id:number; session:number; kind:"text"|"image"; copied:boolean; resolve:(blob:Blob)=>void; reject:(error:Error)=>void;
    outcome:Promise<boolean>; complete:(copied:boolean)=>void; localReady:Promise<void>; localComplete:()=>void;localFailure:(error:Error)=>void;
    write:Promise<void>|null; bytes:Uint8Array<ArrayBuffer>|null; received:number; timer:ReturnType<typeof setTimeout> };
  let copy:Copy|null=null;

  function send(command:Record<string,unknown>) {
    if(token&&command.mirror!==true&&["clipboard_paste_text","clipboard_set"].includes(String(command.type))) {
      cancelMirror();mirroredKey=null;
      if(command.order===undefined)command={...command,order:++sequence};
    }
    // Committed text and ordinary text paste must retain their position among
    // key events. The bulk socket can overtake a preceding Backspace/selection.
    if (!token || command.type === "clipboard_paste_text") return options.send(command);
    if(!options.ready())return false;
    if(!socket||socket.readyState!==WebSocket.OPEN){if(outbox.length>=16)return false;outbox.push(command);return true;}
    if (socket.bufferedAmount>262144) {socket.close(4001,"Clipboard backpressure");return false;}
    try {socket.send(JSON.stringify(command));return true;}catch{return false;}
  }
  function cancelMirror(notify=true) {
    mirrorEpoch++;
    if(!mirrorBusy)return;
    mirrorBusy=false;busy=false;operation++;
    const id=mirrorId;mirrorId=null;
    if(pending&&pending.id===id){const old=pending;pending=null;clearTimeout(old.timer);old.reject(new Error("Clipboard synchronization superseded."));}
    if(notify&&id!==null)send({type:"clipboard_cancel",id});
  }
  async function readLocal():Promise<Blob|string|null> {
    if(navigator.clipboard?.read) {
      const items=await navigator.clipboard.read();
      for(const item of items){const type=["image/png","image/jpeg","image/webp"].find(type=>item.types.includes(type));if(type)return item.getType(type);}
      for(const item of items)if(item.types.includes("text/plain"))return (await item.getType("text/plain")).text();
      return null;
    }
    return navigator.clipboard.readText();
  }
  async function localKey(value:Blob|string|null) {
    if(typeof value==="string"||value===null)return "text:"+(value??"");
    if(value.size>MAX_IMAGE)throw new Error("Clipboard image exceeds 8 MiB.");
    const hash=await crypto.subtle.digest("SHA-256",await value.arrayBuffer());
    return value.type+":"+Array.from(new Uint8Array(hash),byte=>byte.toString(16).padStart(2,"0")).join("");
  }
  async function mirrorLocal() {
    if(!synchronize||!token||!localActive||!options.ready()||copy||busy||!navigator.clipboard||!navigator.permissions)return;
    const epoch=++mirrorEpoch,session=options.session();
    let current:number|undefined;
    try {
      const permission=await navigator.permissions.query({name:"clipboard-read" as PermissionName});
      if(permission.state!=="granted")return;
      const value=await readLocal(),key=await localKey(value);
      if(epoch!==mirrorEpoch||session!==options.session()||!localActive||!options.ready()||copy||busy||key===mirroredKey)return;
      busy=mirrorBusy=true;current=++operation;const id=++sequence;mirrorId=id;
      const showProgress=value instanceof Blob&&value.size>CHUNK;
      if(showProgress)options.message("Synchronizing local clipboard…");
      await request({type:"clipboard_mirror_start",mirror:true},id,0);
      if(epoch!==mirrorEpoch||current!==operation||session!==options.session()||!options.ready())return;
      if(value instanceof Blob)await transfer(value,session,false,current,id);
      else await request({type:"clipboard_set",text:value??"",mirror:true},id,1);
      if(epoch===mirrorEpoch&&current===operation){mirroredKey=key;if(showProgress)options.message("Local clipboard synchronized.");}
    } catch(error) {
      if(epoch===mirrorEpoch&&session===options.session()&&options.ready()&&current!==undefined)options.message(`Clipboard synchronization: ${error instanceof Error?error.message:String(error)}`);
    } finally {
      if(current===operation){busy=mirrorBusy=false;mirrorId=null;}
    }
  }
  navigator.clipboard?.addEventListener?.("clipboardchange",()=>{void mirrorLocal();},{signal:options.signal});

  function cancelCopy(error?:Error) {
    if(!copy)return;
    if(error)copyError=error;
    const old=copy;copy=null;clearTimeout(old.timer);old.reject(new Error("Copy cancelled."));old.complete(false);old.localFailure(new Error("Copy cancelled."));
  }
  function connect(next:string|null,sync=false) {
    socketEpoch++;const epoch=socketEpoch;
    if(retry!==null)clearTimeout(retry);retry=null;
    if(socket){socket.onmessage=socket.onclose=socket.onerror=null;socket.close();socket=null;}
    token=next;synchronize=sync;
    outbox=[];
    if(!next)return;
    const endpoint=new URL("./clipboard",location.href);endpoint.protocol=location.protocol==="https:"?"wss:":"ws:";endpoint.searchParams.set("session",next);
    let delay=250;
    function open() {
      if(epoch!==socketEpoch||!options.ready())return;
      const current=new WebSocket(endpoint);current.binaryType="arraybuffer";socket=current;
      current.onopen=()=>{if(socket===current){const queued=outbox;outbox=[];for(const command of queued)if(!send(command))break;void mirrorLocal();}};
      current.onmessage=event=>{
        if(epoch!==socketEpoch||socket!==current)return;
        delay=250;
        if(typeof event.data==="string") {try{const value=JSON.parse(event.data);if(!receive(value)&&value.type==="error")options.message(`Clipboard: ${String(value.message)}`);}catch{}}
        else if(event.data instanceof ArrayBuffer)receiveBinary(event.data);
      };
      current.onerror=()=>{if(socket===current)current.close();};
      current.onclose=()=>{
        if(epoch!==socketEpoch||socket!==current)return;
        socket=null;outbox=[];cancelMirror(false);cancelCopy(new Error("Clipboard connection lost during copy."));
        if(pending){const old=pending;pending=null;clearTimeout(old.timer);old.reject(new Error("Clipboard connection lost."));}
        retry=setTimeout(()=>{retry=null;open();},delay);delay=Math.min(5000,delay*2);
      };
    }
    open();
  }

  function beginCopy(id:number,gesture:boolean) {
    if(gesture)cancelMirror();
    cancelCopy();copyError=null;
    let resolve!:(blob:Blob)=>void,reject!:(error:Error)=>void,complete!:(value:boolean)=>void;
    const content=new Promise<Blob>((a,b)=>{resolve=a;reject=b;});
    // A rejected image/text alternative must never become an unhandled rejection.
    void content.catch(()=>{});
    const outcome=new Promise<boolean>(done=>{complete=done;});
    let localComplete!:()=>void,localFailure!:(error:Error)=>void;
    const localReady=new Promise<void>((a,b)=>{localComplete=a;localFailure=b;});void localReady.catch(()=>{});
    const current:Copy={id,session:options.session(),kind:"text",copied:false,resolve,reject,complete,outcome,localReady,localComplete,localFailure,write:null,bytes:null,received:0,
      timer:setTimeout(()=>{if(copy===current)cancelCopy(new Error("Clipboard copy timed out."));},15000)};
    copy=current;
    if(gesture&&typeof ClipboardItem!=="undefined"&&navigator.clipboard?.write) {
      // Start the write during the copy gesture; Safari requires the gesture
      // even when the remotely supplied text arrives asynchronously.
      try { current.write=navigator.clipboard.write([new ClipboardItem({"text/plain":content})]); }
      catch(error) { current.write=Promise.reject(error); }
      void current.write.catch(()=>{});
    }
    return current;
  }
  function copyKey(code:number,chord?:number[],semantic=false) {
    if(!synchronize||!token||!options.ready())return false;
    const current=beginCopy(++sequence,true);
    // The key travels on the input connection, in order with its modifiers and
    // key-up. Clipboard data comes back on the independent bulk connection.
    if(!options.send({type:"clipboard_copy_key",id:current.id,code,...(chord?{chord}: {}),...(semantic?{semantic:true}:{})})){cancelCopy();return false;}
    return true;
  }
  async function finishCopy(current:Copy,blob:Blob) {
    if(copy!==current||current.session!==options.session()||!options.ready())return;
    cancelMirror();
    clearTimeout(current.timer);current.copied=true;current.complete(true);
    try {
      if(current.kind==="text") {
        current.resolve(blob);
        if(current.write)await current.write;
        else await navigator.clipboard.writeText(await blob.text());
      } else {
        current.reject(new Error("Image copy."));
        if(typeof ClipboardItem==="undefined"||!navigator.clipboard?.write)throw new Error("Image clipboard write is unavailable.");
        await navigator.clipboard.write([new ClipboardItem({"image/png":blob})]);
      }
      mirroredKey=await localKey(current.kind==="text"?await blob.text():blob);
      current.localComplete();
      if(copy===current){copy=null;options.message("Remote clipboard copied to this device.");}
    } catch(error) {
      const failure=error instanceof Error?error:new Error(String(error));current.localFailure(failure);
      if(copy===current){copyError=failure;copy=null;options.message(`Clipboard synchronization failed: ${failure.message}`);}
    }
  }
  function receiveBinary(buffer:ArrayBuffer) {
    const data=new Uint8Array(buffer);if(data[0]!==5||data.length<13)return;
    const view=new DataView(buffer);const id=Number(view.getBigUint64(1,true)),offset=view.getUint32(9,true);
    if(!copy||copy.id!==id||!copy.bytes||offset!==copy.received||copy.received+data.length-13>copy.bytes.length)return;
    copy.bytes.set(data.subarray(13),offset);copy.received+=data.length-13;
    clearTimeout(copy.timer);const current=copy;current.timer=setTimeout(()=>{if(copy===current)cancelCopy(new Error("Clipboard copy timed out."));},15000);
  }

  function reset() {
    cancelMirror(false);mirroredKey=null;
    cancelCopy();copyError=null;connect(null);
    operation++;
    busy = false;
    if (pending) {
      clearTimeout(pending.timer);
      pending.reject(new Error("Clipboard transfer interrupted."));
      pending = null;
    }
  }

  function receive(value: Record<string, unknown>) {
    if(value.type==="clipboard_copy_started") {
      if(copy&&copy.id<4_000_000_000_000_000)return true;
      if(localActive&&synchronize&&options.ready()&&Number.isSafeInteger(value.id)&&Number(value.id)>=4_000_000_000_000_000)beginCopy(Number(value.id),false);
      return true;
    }
    if(value.type==="clipboard_copy"||value.type==="clipboard_copy_begin"||value.type==="clipboard_copy_end") {
      const current=copy;if(!current||value.id!==current.id)return true;
      if(value.type==="clipboard_copy_begin") {
        if(value.mime!=="image/png"||!Number.isSafeInteger(value.size)||Number(value.size)<1||Number(value.size)>MAX_IMAGE){cancelCopy(new Error("Invalid clipboard image."));return true;}
        current.kind="image";current.bytes=new Uint8Array(Number(value.size));current.copied=true;current.complete(true);current.reject(new Error("Image copy."));
      } else if(value.type==="clipboard_copy_end") {
        if(current.bytes&&current.received===current.bytes.length)void finishCopy(current,new Blob([current.bytes],{type:"image/png"}));else cancelCopy(new Error("Incomplete clipboard image."));
      } else if(value.copied===true&&typeof value.text==="string")void finishCopy(current,new Blob([value.text],{type:"text/plain"}));
      else {
        current.copied=value.copied===true;current.complete(current.copied);current.reject(new Error(typeof value.error==="string"?value.error:"No copied selection."));
        if(current.copied||value.error){copyError=new Error(String(value.error??"Clipboard synchronization failed."));current.localFailure(copyError);}else current.localComplete();
        clearTimeout(current.timer);copy=null;if(value.error)options.message(`Clipboard: ${String(value.error)}`);
        else void mirrorLocal();
      }
      return true;
    }
    if (value.type !== "clipboard_ack") return false;
    if (pending && value.id === pending.id && value.step === pending.step) {
      const request = pending;
      pending = null;
      clearTimeout(request.timer);
      if (typeof value.error === "string") request.reject(new Error(value.error));
      else request.resolve();
    }
    return true;
  }

  function request(command: Record<string, unknown>, id: number, step: number) {
    return new Promise<void>((resolve, reject) => {
      const timer = setTimeout(() => { pending = null; reject(new Error("Clipboard transfer timed out.")); }, 10000);
      pending = { id, step, resolve, reject, timer };
      if (!send({ ...command, id, step })) reset();
    });
  }

  async function png(blob: Blob) {
    if (blob.size > MAX_IMAGE) throw new Error("Clipboard image exceeds 8 MiB.");
    if (blob.type === "image/png") return blob;
    const bitmap = await createImageBitmap(blob);
    try {
      if (bitmap.width > 8192 || bitmap.height > 8192 || bitmap.width * bitmap.height > 33554432) throw new Error("Clipboard image dimensions are too large.");
      const canvas = document.createElement("canvas");
      canvas.width = bitmap.width; canvas.height = bitmap.height;
      canvas.getContext("2d")!.drawImage(bitmap, 0, 0);
      return await new Promise<Blob>((resolve, reject) => canvas.toBlob(value => value ? resolve(value) : reject(new Error("Could not read clipboard image.")), "image/png"));
    } finally { bitmap.close(); }
  }

  async function transfer(blob: Blob, session: number, shift: boolean, current=operation, mirroredId?:number) {
    if (!options.images()) throw new Error("This server does not support image clipboard transfer.");
    const image = await png(blob);
    if (image.size > MAX_IMAGE) throw new Error("Clipboard image exceeds 8 MiB.");
    const bytes = new Uint8Array(await image.arrayBuffer());
    if (session !== options.session() || !options.ready()) return;
    if(current!==operation)return;
    const id = mirroredId??++sequence;
    const flags=mirroredId===undefined?{}:{mirror:true};
    if(mirroredId===undefined)options.release();
    await request({ type: "clipboard_image_start", size: bytes.length,...flags }, id, 0);
    let step = 1;
    for (let offset = 0; offset < bytes.length; offset += CHUNK) {
      if (current!==operation || session !== options.session() || !options.ready()) return;
      const data = btoa(String.fromCharCode(...bytes.subarray(offset, offset + CHUNK)));
      await request({ type: "clipboard_image_chunk", data,...flags }, id, step++);
    }
    if (current===operation && session === options.session() && options.ready()) {
      await request({ type: mirroredId===undefined?"clipboard_image_paste":"clipboard_image_set", shift,...flags }, id, step);
      if(mirroredId===undefined)options.message("Clipboard image pasted.");
    }
  }

  async function perform(read: () => Promise<Blob | string | null>, shift=options.shift?.()??false) {
    if (busy || !options.ready()) return;
    busy = true;
    const current = ++operation;
    const session = options.session();
    try {
      const value = await read();
      if (session !== options.session() || !options.ready()) return;
      if (typeof value === "string") options.text(value,shift);
      else if (value) await transfer(value, session, shift,current);
      else options.message("No text or image is available in the clipboard.");
    } catch (error) {
      if (session === options.session() && options.ready()) options.message(`Clipboard: ${error instanceof Error ? error.message : String(error)}`);
    } finally { if (current === operation) busy = false; }
  }

  return {
    receive, reset, connect, copyKey, send,
    copying:()=>copy!==null&&copy.id<4_000_000_000_000_000,
    localFocusLeft:()=>{localActive=false;cancelMirror();cancelCopy();copyError=null;},
    localFocusEntered:()=>{localActive=true;void mirrorLocal();},
    localChanged:()=>{cancelCopy();copyError=null;cancelMirror();setTimeout(()=>{void mirrorLocal();},0);},
    paste(event: ClipboardEvent) {
      cancelMirror();
      event.preventDefault();
      const items = Array.from(event.clipboardData?.items ?? []);
      const image = items.find(item => item.kind === "file" && ["image/png", "image/jpeg", "image/webp"].includes(item.type))?.getAsFile();
      const text = event.clipboardData?.getData("text/plain") ?? "";
      const current=copy,shift=options.shift?.()??false,session=options.session();
      const paste=async()=>{
        const copied=current?await current.outcome:false;
        if(!options.ready()||session!==options.session())return;
        if(copied&&current){
          try{await current.localReady;}catch{options.message("Clipboard synchronization failed; stale content was not pasted.");return;}
          return this.read(true,shift);
        }
        if(copyError){options.message("Clipboard synchronization failed; stale content was not pasted.");return;}
        if(image)return perform(async()=>image);
        options.text(text,shift);
      };
      if(current||copyError)return paste();
      if (image) return perform(async () => image);
      if (options.ready()) options.text(text,shift);
    },
    read(forceLocal=false,shift=options.shift?.()??false):Promise<void> {
      cancelMirror();
      const current=copy,session=options.session();
      if(!forceLocal&&(current||copyError)) {
        return (async()=>{const copied=current?await current.outcome:false;if(!options.ready()||session!==options.session())return;
          if(copied&&current){try{await current.localReady;}catch{options.message("Clipboard synchronization failed; stale content was not pasted.");return;}}
          if(copyError){options.message("Clipboard synchronization failed; stale content was not pasted.");return;}
          await this.read(true,shift);
        })();
      }
      if(forceLocal)copyError=null;
      return perform(readLocal,shift);
    },
  };
}
