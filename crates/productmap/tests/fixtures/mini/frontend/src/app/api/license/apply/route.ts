import { callUpstream } from "@/src/lib/api/upstream";
export async function POST(req) { return callUpstream(req, "/license/apply", { method: "POST" }); }
