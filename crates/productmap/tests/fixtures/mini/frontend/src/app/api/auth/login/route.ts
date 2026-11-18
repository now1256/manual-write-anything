import { BACKEND_ORIGIN } from "@/src/app/api/constant";
export async function POST(req) { return fetch(`${BACKEND_ORIGIN}/auth/login`, { method: "POST" }); }
