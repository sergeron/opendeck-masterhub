// Wspólna obsługa panelu ustawień (protokół Stream Deck / OpenAction).
// Każde pole formularza z atrybutem data-key jest zapisywane w ustawieniach akcji.

let socket = null;
let actionInfo = null;

function readForm() {
	const settings = {};
	for (const el of document.querySelectorAll("[data-key]")) {
		const key = el.dataset.key;
		if (el.type === "checkbox") settings[key] = el.checked;
		else if (el.type === "range" || el.type === "number" || el.dataset.number !== undefined) settings[key] = Number(el.value);
		else settings[key] = el.value;
	}
	return settings;
}

function fillForm(settings) {
	for (const el of document.querySelectorAll("[data-key]")) {
		const value = settings[el.dataset.key];
		if (value === undefined) continue;
		if (el.type === "checkbox") el.checked = Boolean(value);
		else el.value = String(value);
	}
}

function save() {
	if (typeof onFormChange === "function") onFormChange();
	if (!socket) return;
	socket.send(JSON.stringify({ event: "setSettings", context: actionInfo.context, payload: readForm() }));
}

function connectElgatoStreamDeckSocket(port, uuid, registerEvent, info, action) {
	actionInfo = JSON.parse(action);
	fillForm(actionInfo.payload.settings || {});
	if (typeof onFormChange === "function") onFormChange();

	socket = new WebSocket("ws://localhost:" + port);
	I18N.attach(socket, uuid);
	// Wiadomości od pluginu (sendToPropertyInspector): język obsługuje i18n.js, resztę strona
	// w onPluginMessage.
	socket.onmessage = (e) => {
		const msg = JSON.parse(e.data);
		if (msg.event !== "sendToPropertyInspector" || I18N.handle(msg.payload)) return;
		if (typeof onPluginMessage === "function") onPluginMessage(msg.payload);
	};
	socket.onopen = () => {
		socket.send(JSON.stringify({ event: registerEvent, uuid }));
		// Zapisujemy od razu, żeby wartości domyślne trafiły do pluginu.
		save();
	};

	for (const el of document.querySelectorAll("[data-key]")) {
		el.addEventListener(el.type === "text" ? "change" : "input", save);
	}
}
