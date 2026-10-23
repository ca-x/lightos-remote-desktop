import { create, toBinary } from "@bufbuild/protobuf";
import { CommandSchema, type Command } from "./gen/wire_pb.js";

export const framesProtocol = "lightos.frames.v1";
export const protobufProtocol = "lightos.protobuf.v1";

export function requestedProtocols(href: string): string[] {
  const mode = new URL(href).searchParams.get("wire");
  return mode === "protobuf" ? [protobufProtocol] : mode === "frames" ? [framesProtocol] : [];
}

// Low-frequency setup, text, clipboard and HTTP APIs keep their existing format.
export function encodeBinaryCommand(command: Record<string, unknown>): Uint8Array<ArrayBuffer> | null {
  let kind: Command["kind"];
  switch (command.type) {
    case "frame_ack":
      if (!Number.isSafeInteger(command.id) || Number(command.id) <= 0) return null;
      kind = { case: "frameAck", value: BigInt(command.id as number) }; break;
    case "move": case "scroll":
      kind = { case: command.type, value: { $typeName: "lightos.wire.v1.Point", x: command.x as number, y: command.y as number } }; break;
    case "button":
      kind = { case: "button", value: { $typeName: "lightos.wire.v1.Button", button: command.button as number, pressed: command.pressed as boolean } }; break;
    case "key":
      kind = { case: "key", value: { $typeName: "lightos.wire.v1.Key", code: command.code as number, pressed: command.pressed as boolean } }; break;
    case "release": kind = { case: "release", value: true }; break;
    default: return null;
  }
  return toBinary(CommandSchema, create(CommandSchema, { kind })) as Uint8Array<ArrayBuffer>;
}

export type VideoHeader = { kind: "jpeg" | "h264"; id: number | null; offset: number; timestamp: number; key: boolean };
export function videoHeader(data: ArrayBuffer, inlineIds: boolean): VideoHeader | null {
  if (data.byteLength > 16777216 || data.byteLength < 2) return null;
  const view = new DataView(data);
  const kind = view.getUint8(0);
  if (![1, 3, 5, 6].includes(kind)) return null;
  const inline = kind === 5 || kind === 6;
  if (inline && !inlineIds) return null;
  const h264 = kind === 3 || kind === 6;
  const offset = (h264 ? 10 : 1) + (inline ? 8 : 0);
  if (data.byteLength <= offset || (h264 && view.getUint8(1) > 1)) return null;
  const id = inline ? Number(view.getBigUint64(h264 ? 10 : 1, true)) : null;
  if (id !== null && (!Number.isSafeInteger(id) || id <= 0)) return null;
  const timestamp = h264 ? Number(view.getBigUint64(2, true)) : 0;
  if (!Number.isSafeInteger(timestamp)) return null;
  return { kind: h264 ? "h264" : "jpeg", id, offset, timestamp, key: h264 && view.getUint8(1) === 1 };
}
