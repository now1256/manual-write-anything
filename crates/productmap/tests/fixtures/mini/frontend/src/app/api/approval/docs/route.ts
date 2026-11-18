import { callUpstream } from "@/src/lib/api/upstream";
export async function GET(req) { return callUpstream(req, "/approval/docs"); }
