export default function Login() { fetch("/api/auth/login", { method: "POST" }); return null; }
