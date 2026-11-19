"use client";
import { useSystems, useDeleteSystems } from "@/src/lib/hooks/queries";
import { Can } from "@/src/app/components/can";
import { useHasPermission } from "@/src/lib/contexts/user-context";
import { RegisterModal } from "./_components/register-modal";
export default function ConnectionsPage() {
  const canEdit = useHasPermission("ORD_EDIT");
  const { data } = useSystems();
  const del = useDeleteSystems();
  return (
    <div>
      <Can permission="ORD_CREATE"><RegisterModal /></Can>
    </div>
  );
}
