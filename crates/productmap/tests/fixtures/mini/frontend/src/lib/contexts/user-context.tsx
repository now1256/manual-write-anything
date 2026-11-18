import { apiFetch } from "@/src/lib/api/client";
export const useCurrentUser = () => { apiFetch("/api/auth/me"); return { has: () => true }; };
export const useHasPermission = (code) => true;
