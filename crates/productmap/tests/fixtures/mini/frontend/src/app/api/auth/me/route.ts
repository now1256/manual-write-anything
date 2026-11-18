export async function GET(req) { const auth = await requireApiAuth(req); return auth; }
