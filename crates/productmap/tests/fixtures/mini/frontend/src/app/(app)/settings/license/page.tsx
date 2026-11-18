import { apiFetch } from "@/src/lib/api/client";
export default function LicensePage() { apiFetch("/api/license/apply", { method: "POST" }); return null; }
