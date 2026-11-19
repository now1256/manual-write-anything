export interface CommandDef {
  name: string;
  desc: string;
  /** Runs on the server (otherwise handled in the UI). */
  remote: boolean;
}

export const COMMANDS: CommandDef[] = [
  { name: "map", desc: "코드에서 제품 지도를 다시 만듭니다", remote: true },
  { name: "adapter", desc: "코드를 읽고 제품 어댑터를 만든 뒤 제품 지도로 확인합니다 (1~2분)", remote: true },
  { name: "shot", desc: "메뉴 화면을 모두 촬영합니다 (2회 비교, 약 5분)", remote: true },
  { name: "gen", desc: "기능의 코드를 읽어 사실을 뽑습니다 (1~2분)", remote: true },
  { name: "sync", desc: "git 소스를 클론하거나 최신으로 맞춥니다", remote: true },
  { name: "browser", desc: "촬영용 브라우저(Chromium)를 설치합니다", remote: true },
  { name: "describe", desc: "기능의 기존 매뉴얼 캡처마다 장면 설명을 만듭니다 (기존 매뉴얼 탭)", remote: true },
  { name: "shotref", desc: "기존 매뉴얼의 장면을 다시 찍고 대조합니다 (기존 매뉴얼 탭)", remote: true },
  { name: "shotdoc", desc: "매뉴얼의 그림 자리를 지금 화면으로 찍습니다 (④ 편집·버전)", remote: true },
  { name: "docground", desc: "문서의 문단마다 코드 사실과 대조해 근거를 확인합니다 (④ 편집·버전)", remote: true },
  { name: "docmake", desc: "기존 매뉴얼 + 고른 그림 + 코드 기준 제안으로 매뉴얼을 만듭니다 (③ 매뉴얼 만들기)", remote: true },
  { name: "docref", desc: "기존 매뉴얼(PDF)을 편집하는 문서로 옮깁니다 (새 매뉴얼 탭)", remote: true },
  { name: "docupdate", desc: "문서를 코드와 비교해 갱신 제안을 만듭니다 (새 매뉴얼 탭)", remote: true },
  { name: "doctor", desc: "설치·로그인·소스 상태를 점검합니다", remote: true },
  { name: "auth", desc: "로그인 프로파일 상태를 봅니다", remote: true },
  { name: "clear", desc: "대화를 지웁니다", remote: false },
  { name: "help", desc: "명령과 단축키를 봅니다", remote: false },
];

