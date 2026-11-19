import { apiFetch } from "@/src/lib/api/client";
export const fetchSystems = (db: string) => apiFetch(`/api/sales/accounts?db=${encodeURIComponent(db)}`);
export const fetchSystem = (id: string, db: string) => apiFetch(`/api/sales/accounts/${id}?db=${db}`);
export const createSystem = (db: string) => apiFetch(`/api/sales/accounts?db=${db}`, { method: "POST" });
export const deleteSystems = (db: string) => apiFetch(`/api/sales/accounts?db=${db}`, { method: "DELETE" });
export const testSystem = (db: string) => apiFetch(`/api/sales/accounts/test?db=${db}`, { method: "POST" });
