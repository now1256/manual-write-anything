import { fetchSystems, deleteSystems, createSystem } from "@/src/lib/api/sales/accounts/fetcher";
export const useSystems = () => fetchSystems("db");
export const useDeleteSystems = () => deleteSystems("db");
export const useCreateSystem = () => createSystem("db");
