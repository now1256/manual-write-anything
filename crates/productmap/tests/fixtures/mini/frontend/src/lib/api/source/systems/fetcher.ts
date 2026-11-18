import { apiFetch } from "@/src/lib/api/client";
export const fetchSystems = (db: string) => apiFetch(`/api/source/systems?db=${encodeURIComponent(db)}`);
export const fetchSystem = (id: string, db: string) => apiFetch(`/api/source/systems/${id}?db=${db}`);
export const createSystem = (db: string) => apiFetch(`/api/source/systems?db=${db}`, { method: "POST" });
export const deleteSystems = (db: string) => apiFetch(`/api/source/systems?db=${db}`, { method: "DELETE" });
export const testSystem = (db: string) => apiFetch(`/api/source/systems/test?db=${db}`, { method: "POST" });
