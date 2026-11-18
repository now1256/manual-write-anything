import { apiFetch } from "@/src/lib/api/client";
export const useApprovals = () => apiFetch("/api/approval/docs");
