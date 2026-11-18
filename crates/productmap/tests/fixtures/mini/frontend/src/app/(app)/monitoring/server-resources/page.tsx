import { useStream } from "@/src/lib/hooks/use-stream";
const STREAM = "/api/monitoring/stream";
export default function Page() { useStream(STREAM); return null; }
