PERMISSION_NAMES: dict[str, str] = {
    "QRY_MODEL_VIEW": "NLQ > 시맨틱 모델 > 시맨틱 모델 > 목록 조회",
    "CFG_ACCESS": "설정 > 사용자 및 권한 > 접근",
    "SRC_LINK_CREATE": "소스 > 연결 관리 > 원천 시스템 > 등록",
    "SRC_LINK_EDIT": "소스 > 연결 관리 > 원천 시스템 > 수정",
    "SRC_LINK_VIEW": "소스 > 연결 관리 > 원천 시스템 > 목록 조회",
    "OPS_ACCESS": "설정 > 시스템 > 접근",
}

FREE_MENUS: frozenset[str] = frozenset({"설정", "모니터링"})
