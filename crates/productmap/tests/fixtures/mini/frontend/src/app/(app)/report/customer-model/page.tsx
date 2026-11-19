import { listModels, modelSummary } from "@/src/lib/api/report/customer-model";
export default function Page() { listModels("db"); modelSummary("db"); return null; }
