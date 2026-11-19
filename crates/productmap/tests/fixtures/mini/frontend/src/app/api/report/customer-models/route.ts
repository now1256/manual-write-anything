import { proxyQueryCustomerModels } from "@/src/lib/api/report/proxy";
export async function GET(req) { return proxyQueryCustomerModels(req, "", { fallback: "x" }); }
