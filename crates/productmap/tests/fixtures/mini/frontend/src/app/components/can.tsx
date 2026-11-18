import { useCurrentUser } from "@/src/lib/contexts/user-context";
export function Can({ permission, children }) { const { has } = useCurrentUser(); return has(permission) ? children : null; }
