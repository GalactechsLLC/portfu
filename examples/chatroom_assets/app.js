const joinForm = document.getElementById("join-form");
const chat = document.getElementById("chat");
const statusNode = document.getElementById("status");
const messages = document.getElementById("messages");
const messageForm = document.getElementById("message-form");
const messageInput = document.getElementById("message-input");

let socket = null;

function appendMessage(text) {
  const li = document.createElement("li");
  li.textContent = text;
  messages.appendChild(li);
  messages.scrollTop = messages.scrollHeight;
}

joinForm.addEventListener("submit", (event) => {
  event.preventDefault();
  const form = new FormData(joinForm);
  const name = String(form.get("name") || "").trim();
  if (!name) {
    return;
  }

  const protocol = window.location.protocol === "https:" ? "wss:" : "ws:";
  const wsUrl = `${protocol}//${window.location.host}/ws/chat/${encodeURIComponent(name)}`;
  socket = new WebSocket(wsUrl);

  socket.addEventListener("open", () => {
    statusNode.textContent = `Connected as ${name}`;
    joinForm.classList.add("hidden");
    chat.classList.remove("hidden");
    messageInput.focus();
  });

  socket.addEventListener("message", (event) => {
    appendMessage(String(event.data));
  });

  socket.addEventListener("close", () => {
    statusNode.textContent = "Disconnected";
  });

  socket.addEventListener("error", () => {
    statusNode.textContent = "Connection error";
  });
});

messageForm.addEventListener("submit", (event) => {
  event.preventDefault();
  if (!socket || socket.readyState !== WebSocket.OPEN) {
    return;
  }
  const text = messageInput.value.trim();
  if (!text) {
    return;
  }
  socket.send(text);
  messageInput.value = "";
});
