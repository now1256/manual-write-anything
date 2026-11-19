import { faX } from "@fortawesome/free-solid-svg-icons";
export const LICENSE_HREF = "/settings/license";
export const MAIN_NAV: NavItem[] = [
  {
    key: "sales",
    label: "영업",
    icon: faX,
    children: [
      { key: "sales-orders", label: "주문 관리", href: "/sales/orders", permissions: ["ORD_VIEW"] },
      { key: "report-customer-model", label: "고객 모델", href: "/report/customer-model", permissions: ["RPT_MODEL_VIEW"] },
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
