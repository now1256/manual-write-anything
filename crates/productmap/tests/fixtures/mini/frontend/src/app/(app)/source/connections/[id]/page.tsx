import { fetchSystem } from "@/src/lib/api/source/systems/fetcher";
export default function DetailPage() { fetchSystem("1", "db"); return null; }
