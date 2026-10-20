import { createContext, useContext } from "react";
import { invoke } from "@tauri-apps/api/core";
import { open } from "@tauri-apps/plugin-dialog";
import type { Request } from "./types";
export interface Transport {
  request<T>(request: Request): Promise<T>;
  pickRepository(): Promise<string | null>;
  inspectArtifact(
    runId: string,
    path: string,
  ): Promise<{ path: string; accessible: boolean }>;
}
export const nativeTransport: Transport = {
  request: (request) => invoke("daemon", { request }),
  pickRepository: async () => {
    const path = await open({
      directory: true,
      multiple: false,
      title: "Select an existing Git repository",
    });
    return typeof path === "string" ? path : null;
  },
  inspectArtifact: (runId, path) => invoke("inspect_artifact", { runId, path }),
};
export const TransportContext = createContext<Transport>(nativeTransport);
export const useTransport = () => useContext(TransportContext);
