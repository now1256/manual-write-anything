import { callUpstream } from "@/src/lib/api/upstream";
export async function GET(req) { return callUpstream(req, `/systems?db=${db}`); }
export async function POST(req) { return callUpstream(req, `/systems?db=${db}`, { method: "POST" }); }
export async function DELETE(req) { return callUpstream(req, `/systems?db=${db}`, { method: "DELETE" }); }
