import { faX } from "@fortawesome/free-solid-svg-icons";
export const LICENSE_HREF = "/settings/license";
export const MAIN_NAV: NavItem[] = [
  {
    key: "source",
    label: "소스",
    icon: faX,
    children: [
      { key: "source-connections", label: "연결 관리", href: "/source/connections", permissions: ["SRC_LINK_VIEW"] },
      { key: "nlq-semantic-model", label: "시맨틱 모델", href: "/nlq/semantic-model", permissions: ["QRY_MODEL_VIEW"] },
    ],
  },
];
export const ADMIN_NAV: NavItem[] = [
  { key: "system", label: "시스템", icon: faX, children: [{ key: "license", label: "제품 정보", href: LICENSE_HREF, permissions: ["OPS_ACCESS"] }] },
];
export const OPS_NAV: NavItem[] = [
  { key: "server-resources", label: "서버 자원 현황", href: "/monitoring/server-resources", icon: faX },
];
export const REVIEW_NAV: NavItem[] = [
  { key: "approval-pending", label: "결재 대기", href: "/approval/pending", icon: faX },
];
