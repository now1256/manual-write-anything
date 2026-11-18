import { proxyQuerySemanticModels } from "@/src/lib/api/nlq/proxy";
export async function GET(req) { return proxyQuerySemanticModels(req, "/summary", { fallback: "x" }); }
