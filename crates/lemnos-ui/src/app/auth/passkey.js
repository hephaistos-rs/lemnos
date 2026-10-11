// Browser side of passkeys. The server only speaks JSON; the browser API
// (navigator.credentials) is what talks to the fingerprint reader, phone or
// security key.
//
// Pages opt in with data attributes:
//   data-passkey-sign-in   button: sign in as the user in #username
//   data-passkey-register  button: add a passkey, named by #passkey-label
//   data-passkey-error     element that shows what went wrong

async function post(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: { "Content-Type": "application/json" },
    body: JSON.stringify(body ?? {}),
  });
  const data = await response.json();
  if (!response.ok) throw new Error(data.error ?? "Something went wrong.");
  return data;
}

async function signIn() {
  const username = document.querySelector("#username")?.value ?? "";
  if (!username) throw new Error("Enter your username first.");
  const options = await post("/auth/passkey/sign-in/begin", { username });
  const credential = await navigator.credentials.get({
    publicKey: PublicKeyCredential.parseRequestOptionsFromJSON(options.publicKey),
  });
  const result = await post("/auth/passkey/sign-in/finish", credential.toJSON());
  location.href = result.redirect;
}

async function register() {
  const label = document.querySelector("#passkey-label")?.value ?? "";
  const options = await post("/auth/passkey/register/begin");
  const credential = await navigator.credentials.create({
    publicKey: PublicKeyCredential.parseCreationOptionsFromJSON(options.publicKey),
  });
  await post("/auth/passkey/register/finish", { label, credential: credential.toJSON() });
  location.reload();
}

function wire(selector, action) {
  for (const button of document.querySelectorAll(selector)) {
    button.addEventListener("click", async () => {
      const error = document.querySelector("[data-passkey-error]");
      if (error) error.textContent = "";
      try {
        if (!window.PublicKeyCredential?.parseRequestOptionsFromJSON) {
          throw new Error("This browser is too old for passkeys.");
        }
        await action();
      } catch (problem) {
        // NotAllowedError is how the browser reports "cancelled".
        const message =
          problem.name === "NotAllowedError" ? "Cancelled, or no matching passkey on this device." : problem.message;
        if (error) error.textContent = message;
      }
    });
  }
}

wire("[data-passkey-sign-in]", signIn);
wire("[data-passkey-register]", register);
