import { callUpstream } from "@/src/lib/api/upstream";
export async function POST(req) { return callUpstream(req, `/systems/test?db=${db}`, { method: "POST" }); }
