const MAX_IMAGE = 8 * 1024 * 1024;
const CHUNK = 48 * 1024;

interface ClipboardOptions {
  send: (command: Record<string, unknown>) => boolean;
  ready: () => boolean;
  session: () => number;
  images: () => boolean;
  release: () => void;
  text: (value: string) => void;
  message: (value: string) => void;
  shift?: () => boolean;
}

export function createClipboard(options: ClipboardOptions) {
  let sequence = 0;
  let busy = false;
  let operation = 0;
  let pending: { id: number; step: number; resolve: () => void; reject: (error: Error) => void; timer: ReturnType<typeof setTimeout> } | null = null;

  function reset() {
    operation++;
    busy = false;
    if (pending) {
      clearTimeout(pending.timer);
      pending.reject(new Error("Clipboard transfer interrupted."));
      pending = null;
    }
  }

  function receive(value: Record<string, unknown>) {
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
      if (!options.send({ ...command, id, step })) reset();
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

  async function transfer(blob: Blob, session: number, shift: boolean) {
    if (!options.images()) throw new Error("This server does not support image clipboard transfer.");
    const image = await png(blob);
    if (image.size > MAX_IMAGE) throw new Error("Clipboard image exceeds 8 MiB.");
    const bytes = new Uint8Array(await image.arrayBuffer());
    if (session !== options.session() || !options.ready()) return;
    const id = ++sequence;
    options.release();
    await request({ type: "clipboard_image_start", size: bytes.length }, id, 0);
    let step = 1;
    for (let offset = 0; offset < bytes.length; offset += CHUNK) {
      if (session !== options.session() || !options.ready()) return;
      const data = btoa(String.fromCharCode(...bytes.subarray(offset, offset + CHUNK)));
      await request({ type: "clipboard_image_chunk", data }, id, step++);
    }
    if (session === options.session() && options.ready()) {
      await request({ type: "clipboard_image_paste", shift }, id, step);
      options.message("Clipboard image pasted.");
    }
  }

  async function perform(read: () => Promise<Blob | string | null>) {
    if (busy || !options.ready()) return;
    busy = true;
    const current = ++operation;
    const session = options.session();
    const shift = options.shift?.() ?? false;
    try {
      const value = await read();
      if (session !== options.session() || !options.ready()) return;
      if (typeof value === "string") options.text(value);
      else if (value) await transfer(value, session, shift);
      else options.message("No text or image is available in the clipboard.");
    } catch (error) {
      if (session === options.session() && options.ready()) options.message(`Clipboard: ${error instanceof Error ? error.message : String(error)}`);
    } finally { if (current === operation) busy = false; }
  }

  return {
    receive, reset,
    paste(event: ClipboardEvent) {
      event.preventDefault();
      const items = Array.from(event.clipboardData?.items ?? []);
      const image = items.find(item => item.kind === "file" && ["image/png", "image/jpeg", "image/webp"].includes(item.type))?.getAsFile();
      const text = event.clipboardData?.getData("text/plain") ?? "";
      if (image) return perform(async () => image);
      if (options.ready()) options.text(text);
    },
    read() {
      return perform(async () => {
        if (navigator.clipboard?.read) {
          const items = await navigator.clipboard.read();
          for (const item of items) {
            const type = ["image/png", "image/jpeg", "image/webp"].find(type => item.types.includes(type));
            if (type) return item.getType(type);
          }
          for (const item of items) if (item.types.includes("text/plain")) return (await item.getType("text/plain")).text();
          return null;
        }
        return navigator.clipboard.readText();
      });
    },
  };
}
