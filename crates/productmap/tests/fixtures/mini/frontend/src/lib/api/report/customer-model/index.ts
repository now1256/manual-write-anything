import { apiFetch } from "@/src/lib/api/client";
const BASE = "/api/report/customer-models";
const url = (db: string, path = "") => `${BASE}${path}?db=${db}`;
export const listModels = (db: string) => apiFetch(url(db));
export const modelSummary = (db: string) => apiFetch(url(db, "/summary"));
