import { describe, expect, it } from "vitest";
import vectors from "./fixtures/wire-v1.json";
import { encodeBinaryCommand, framesProtocol, protobufProtocol, requestedProtocols, videoHeader } from "../web/src/wire";

describe("experimental binary wire", () => {
  it("requires explicit selection and preserves the legacy default", () => {
    expect(requestedProtocols("http://localhost/")).toEqual([]);
    expect(requestedProtocols("http://localhost/?wire=garbage")).toEqual([]);
    expect(requestedProtocols("http://localhost/?wire=frames")).toEqual([framesProtocol]);
    expect(requestedProtocols("http://localhost/?wire=protobuf")).toEqual([protobufProtocol]);
  });
  it("matches Rust golden vectors including defaults, releases and large IDs", () => {
    for (const { command, hex } of vectors) {
      expect(Buffer.from(encodeBinaryCommand(command)!).toString("hex")).toBe(hex);
    }
    expect(encodeBinaryCommand({type:"clipboard_set",text:"private"})).toBeNull();
    expect(encodeBinaryCommand({type:"frame_ack",id:Number.MAX_SAFE_INTEGER+1})).toBeNull();
  });
  it("bounds inline headers and keeps video bytes as a view", () => {
    const data = new ArrayBuffer(21);
    const view = new DataView(data);
    view.setUint8(0,6); view.setUint8(1,1);
    view.setBigUint64(2,42n,true); view.setBigUint64(10,1000n,true);
    new Uint8Array(data,18).set([0,0,1]);
    expect(videoHeader(data,false)).toBeNull();
    expect(videoHeader(data,true)).toEqual({kind:"h264",id:1000,offset:18,timestamp:42,key:true});
    expect(new Uint8Array(data,videoHeader(data,true)!.offset).buffer).toBe(data);
    expect(videoHeader(data.slice(0,18),true)).toBeNull();
    view.setBigUint64(10,1n<<53n,true); expect(videoHeader(data,true)).toBeNull();
    view.setBigUint64(10,0n,true); expect(videoHeader(data,true)).toBeNull();
  });
});
