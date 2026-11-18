export interface CommandDef {
  name: string;
  desc: string;
  /** Runs on the server (otherwise handled in the UI). */
  remote: boolean;
}

export const COMMANDS: CommandDef[] = [
  { name: "map", desc: "코드에서 제품 지도를 다시 만듭니다", remote: true },
  { name: "shot", desc: "메뉴 화면을 모두 촬영합니다 (2회 비교, 약 5분)", remote: true },
  { name: "gen", desc: "열어 둔 기능의 매뉴얼을 만듭니다 (사실 추출·작성·검증, 수 분)", remote: true },
  { name: "sync", desc: "git 소스를 클론하거나 최신으로 맞춥니다", remote: true },
  { name: "browser", desc: "촬영용 브라우저(Chromium)를 설치합니다", remote: true },
  { name: "doctor", desc: "설치·로그인·소스 상태를 점검합니다", remote: true },
  { name: "auth", desc: "로그인 프로파일 상태를 봅니다", remote: true },
  { name: "clear", desc: "대화를 지웁니다", remote: false },
  { name: "help", desc: "명령과 단축키를 봅니다", remote: false },
];

export const HELP_LINES = [
  ...COMMANDS.map((c) => `/${c.name.padEnd(7)} ${c.desc}`),
  "Enter 보내기 · Shift+Enter 줄바꿈 · / 명령 목록 · Esc 기능 닫기",
];

