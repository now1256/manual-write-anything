PERMISSION_NAMES: dict[str, str] = {
    "RPT_MODEL_VIEW": "분석 > 고객 모델 > 고객 모델 > 목록 조회",
    "CFG_ACCESS": "설정 > 사용자 및 권한 > 접근",
    "ORD_CREATE": "영업 > 주문 관리 > 주문 > 등록",
    "ORD_EDIT": "영업 > 주문 관리 > 주문 > 수정",
    "ORD_VIEW": "영업 > 주문 관리 > 주문 > 목록 조회",
    "OPS_ACCESS": "설정 > 시스템 > 접근",
}

FREE_MENUS: frozenset[str] = frozenset({"설정", "모니터링"})
