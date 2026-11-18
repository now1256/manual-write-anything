import { listModels, modelSummary } from "@/src/lib/api/nlq/semantic-model";
export default function Page() { listModels("db"); modelSummary("db"); return null; }
