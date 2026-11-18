"""needs_permission("CFG_ACCESS") appears only in this docstring."""
@router.post("/license/apply")
async def apply(user: dict = Depends(needs_permission("OPS_ACCESS"))):
    pass
