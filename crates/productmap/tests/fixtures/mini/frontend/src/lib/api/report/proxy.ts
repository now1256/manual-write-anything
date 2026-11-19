import { callUpstream } from "@/src/lib/api/upstream";
async function proxyQuery(base: string, req, path: string, opts) {
  return callUpstream(req, `${base}${path}?db=x`, { method: opts.method ?? "GET" });
}
export const proxyQueryCustomerModels = (req, path: string, opts) => proxyQuery("/report/customer-models", req, path, opts);
