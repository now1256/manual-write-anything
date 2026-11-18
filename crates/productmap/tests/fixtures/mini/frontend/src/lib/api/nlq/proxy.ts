import { callUpstream } from "@/src/lib/api/upstream";
async function proxyQuery(base: string, req, path: string, opts) {
  return callUpstream(req, `${base}${path}?db=x`, { method: opts.method ?? "GET" });
}
export const proxyQuerySemanticModels = (req, path: string, opts) => proxyQuery("/nlq/semantic-models", req, path, opts);
