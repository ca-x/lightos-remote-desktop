import { Blob as NativeBlob } from "node:buffer";
import { afterEach, expect, it, vi } from "vitest";
import { createClipboard } from "../web/src/clipboard";

afterEach(() => vi.unstubAllGlobals());

function harness(autoAck = true) {
  const sent: Record<string, unknown>[] = [];
  let session = 1;
  const text = vi.fn(), message = vi.fn(), release = vi.fn();
  const clipboard = createClipboard({ ready: () => true, session: () => session, images: () => true,
    text, message, release,
    send(command) {
      sent.push(command);
      if (autoAck) queueMicrotask(() => clipboard.receive({ type: "clipboard_ack", id: command.id, step: command.step }));
      return true;
    },
  });
  return { clipboard, sent, text, message, release, nextSession() { session++; clipboard.reset(); } };
}

function paste(blob?: Blob, text = "") {
  return { preventDefault: vi.fn(), clipboardData: { getData: () => text,
    items: blob ? [{ kind: "file", type: blob.type, getAsFile: () => blob }] : [] } } as unknown as ClipboardEvent;
}

it("transfers a clipboard image in bounded acknowledged chunks before requesting remote paste", async () => {
  const h = harness();
  const bytes = new Uint8Array(130000).fill(43);
  await h.clipboard.paste(paste(new NativeBlob([bytes], { type: "image/png" }) as unknown as Blob, "image alt text"));
  expect(h.release).toHaveBeenCalledOnce();
  expect(h.sent.map(c => c.type)).toEqual(["clipboard_image_start", "clipboard_image_chunk", "clipboard_image_chunk", "clipboard_image_chunk", "clipboard_image_paste"]);
  expect(h.sent.map(c => c.step)).toEqual([0, 1, 2, 3, 4]);
  const decoded = h.sent.filter(c => c.type === "clipboard_image_chunk").map(c => Buffer.from(c.data as string, "base64"));
  expect(Buffer.concat(decoded)).toEqual(Buffer.from(bytes));
  expect(h.text).not.toHaveBeenCalled();
  expect(h.message).toHaveBeenLastCalledWith("Clipboard image pasted.");
});

it("keeps text paste synchronous and reads images in preference to text from the explicit paste button", async () => {
  const h = harness();
  h.clipboard.paste(paste(undefined, "本机中文"));
  expect(h.text).toHaveBeenCalledWith("本机中文",false);
  const blob = new NativeBlob([new Uint8Array([1, 2, 3])], { type: "image/png" }) as unknown as Blob;
  vi.stubGlobal("navigator", { clipboard: { read: async () => [{ types: ["text/plain", "image/png"], getType: async () => blob }] } });
  await h.clipboard.read();
  expect(h.sent.at(-1)?.type).toBe("clipboard_image_paste");
});

it("does not send stale clipboard reads or continue an image upload after reconnect", async () => {
  const h = harness();
  let resolve!: (value: string) => void;
  vi.stubGlobal("navigator", { clipboard: { readText: () => new Promise<string>(done => { resolve = done; }) } });
  const reading = h.clipboard.read();
  h.nextSession(); resolve("stale"); await reading;
  expect(h.text).not.toHaveBeenCalled();
  const blocked = harness(false);
  const uploading = blocked.clipboard.paste(paste(new NativeBlob(["png"], { type: "image/png" }) as unknown as Blob));
  await vi.waitFor(() => expect(blocked.sent).toHaveLength(1));
  blocked.nextSession(); await uploading;
  expect(blocked.sent).toHaveLength(1);
});

it("rejects oversized images and reports server rejection instead of pasting old clipboard content", async () => {
  const h = harness();
  await h.clipboard.paste(paste(new NativeBlob([new Uint8Array(8 * 1024 * 1024 + 1)], { type: "image/png" }) as unknown as Blob));
  expect(h.sent).toEqual([]);
  expect(h.message).toHaveBeenLastCalledWith("Clipboard: Clipboard image exceeds 8 MiB.");
  const blocked = harness(false);
  const operation = blocked.clipboard.paste(paste(new NativeBlob(["png"], { type: "image/png" }) as unknown as Blob));
  await vi.waitFor(() => expect(blocked.sent).toHaveLength(1));
  blocked.clipboard.receive({ type: "clipboard_ack", id: 1, step: 0, error: "upload rejected" });
  await operation;
  expect(blocked.sent).toHaveLength(1);
  expect(blocked.message).toHaveBeenLastCalledWith("Clipboard: upload rejected");
});

function localAuthority(writeError=false) {
  const h=harness();let local="Search stale local clipboard";
  vi.stubGlobal("Blob",NativeBlob);
  vi.stubGlobal("ClipboardItem",class {constructor(public data:Record<string,Blob|Promise<Blob>>) {}});
  vi.stubGlobal("WebSocket",class {
    static OPEN=1;readyState=1;bufferedAmount=0;close=vi.fn();
    constructor(public url:URL){}
    send=vi.fn();
  });
  const write=vi.fn(async(items:{data:Record<string,Blob|Promise<Blob>>}[])=>{
    const blob=await items[0].data['text/plain'];
    if(writeError)throw new Error("NotAllowedError");
    local=await blob.text();
  });
  vi.stubGlobal("navigator",{clipboard:{write,writeText:async(text:string)=>{if(writeError)throw new Error("NotAllowedError");local=text;},readText:async()=>local}});
  h.clipboard.connect('a'.repeat(32),true);
  return {...h,write,getLocal:()=>local,setLocal:(text:string)=>{local=text;}};
}

it("keeps committed text on the ordered input connection when the bulk clipboard channel is enabled", () => {
  const h = localAuthority();
  h.clipboard.send({ type: "clipboard_paste_text", text: "中文 123", shift: false });
  expect(h.sent.at(-1)).toMatchObject({ type: "clipboard_paste_text", text: "中文 123", shift: false });
  h.clipboard.reset();
});

it("copies fresh remote text into the local clipboard before a rapid paste can consume stale local data",async()=>{
  const h=localAuthority();
  expect(h.clipboard.copyKey(46)).toBe(true);
  const id=h.sent.at(-1)!.id;
  const pasting=h.clipboard.paste(paste(undefined,h.getLocal()));
  h.clipboard.receive({type:'clipboard_copy',id,copied:true,text:'远端刚复制的中文'});
  await pasting;
  expect(h.getLocal()).toBe('远端刚复制的中文');
  expect(h.text).toHaveBeenCalledWith('远端刚复制的中文',false);
  h.clipboard.reset();
});

it("uses a newer local clipboard value after a completed remote copy instead of a cached remote selection",async()=>{
  const h=localAuthority();h.clipboard.copyKey(46);
  h.clipboard.receive({type:'clipboard_copy',id:h.sent.at(-1)!.id,copied:true,text:'remote selection'});
  await vi.waitFor(()=>expect(h.getLocal()).toBe('remote selection'));
  h.setLocal('new text copied in another Mac app');
  await h.clipboard.paste(paste(undefined,h.getLocal()));
  expect(h.text).toHaveBeenCalledWith('new text copied in another Mac app',false);
  h.clipboard.reset();
});

it("does not paste stale content when the local clipboard rejects remote-copy synchronization",async()=>{
  const h=localAuthority(true);h.clipboard.copyKey(46);
  const pasting=h.clipboard.paste(paste(undefined,h.getLocal()));
  h.clipboard.receive({type:'clipboard_copy',id:h.sent.at(-1)!.id,copied:true,text:'fresh remote selection'});
  await pasting;
  expect(h.text).not.toHaveBeenCalled();
  expect(h.getLocal()).toBe('Search stale local clipboard');
  h.clipboard.reset();
});

it("preserves local clipboard data when Ctrl+C did not change the remote selection, and ignores late replies after disconnect",async()=>{
  const h=localAuthority();h.clipboard.copyKey(46);
  h.clipboard.receive({type:'clipboard_copy',id:h.sent.at(-1)!.id,copied:false});
  await h.clipboard.paste(paste(undefined,h.getLocal()));
  expect(h.getLocal()).toBe('Search stale local clipboard');
  h.clipboard.copyKey(46);const id=h.sent.at(-1)!.id;h.nextSession();
  h.clipboard.receive({type:'clipboard_copy',id,copied:true,text:'late private selection'});
  await Promise.resolve();expect(h.getLocal()).toBe('Search stale local clipboard');
});

it("does not paste a captured native event into a replacement desktop while remote copy is pending",async()=>{
  const h=localAuthority();h.clipboard.copyKey(46);
  const pasting=h.clipboard.paste(paste(undefined,'old desktop selection'));
  h.nextSession();await pasting;
  expect(h.text).not.toHaveBeenCalled();
});

it("synchronizes an application-menu copy and blocks stale paste when observation fails",async()=>{
  const h=localAuthority();
  // A copy made through a remote application's menu has no local key event.
  h.clipboard.receive({type:'clipboard_copy_started',id:4_000_000_000_000_001});
  h.clipboard.receive({type:'clipboard_copy',id:4_000_000_000_000_001,copied:true,text:'menu copy'});
  // No gesture-reserved write: use the ordinary local clipboard write API.
  await vi.waitFor(()=>expect(h.getLocal()).toBe('menu copy'));
  h.clipboard.copyKey(46);
  h.clipboard.receive({type:'clipboard_copy',id:h.sent.at(-1)!.id,copied:false,error:'observer unavailable'});
  await h.clipboard.paste(paste(undefined,'stale'));
  expect(h.text).not.toHaveBeenCalled();
  h.clipboard.reset();
});

it("does not overwrite a newer local copy when the user leaves the remote page during synchronization",async()=>{
  const h=localAuthority();h.clipboard.copyKey(46);const id=h.sent.at(-1)!.id;
  h.clipboard.localFocusLeft();h.setLocal('copied in another local application');
  h.clipboard.receive({type:'clipboard_copy',id,copied:true,text:'late remote selection'});
  await Promise.resolve();expect(h.getLocal()).toBe('copied in another local application');
  h.clipboard.reset();
});
