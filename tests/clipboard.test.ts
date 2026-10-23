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
  expect(h.text).toHaveBeenCalledWith("本机中文");
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
