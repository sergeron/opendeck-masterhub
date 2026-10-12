// Dwujęzyczne panele ustawień (polski / angielski).
// Polski tekst jest w HTML, angielski w atrybucie data-en (placeholder: data-en-placeholder);
// teksty z JavaScriptu przez t("polski", "English"). Język wybiera plugin (sendToPropertyInspector
// z { lang, langSetting }); na dole panelu jest wybór języka zapisywany w ustawieniach globalnych.

const I18N = {
	lang: null,
	setting: "auto",
	socket: null,
	uuid: null,
	listeners: [],

	apply(lang) {
		this.lang = lang === "pl" ? "pl" : "en";
		const pl = this.lang === "pl";
		document.documentElement.lang = this.lang;
		for (const el of document.querySelectorAll("[data-en]")) {
			if (el.dataset.pl === undefined) el.dataset.pl = el.innerHTML;
			el.innerHTML = pl ? el.dataset.pl : el.dataset.en;
		}
		for (const el of document.querySelectorAll("[data-en-placeholder]")) {
			if (el.dataset.plPlaceholder === undefined) el.dataset.plPlaceholder = el.placeholder;
			el.placeholder = pl ? el.dataset.plPlaceholder : el.dataset.enPlaceholder;
		}
		const select = document.getElementById("i18n-language");
		if (select) select.value = this.setting;
		document.body.classList.add("i18n-ready");
		for (const cb of this.listeners) cb(this.lang);
	},

	// Wiadomość od pluginu; zwraca true, jeśli dotyczyła języka.
	handle(payload) {
		if (!payload || !payload.lang) return false;
		this.setting = payload.langSetting || "auto";
		this.apply(payload.lang);
		return true;
	},

	attach(socket, uuid) {
		this.socket = socket;
		this.uuid = uuid;
	},

	onChange(cb) {
		this.listeners.push(cb);
	},

	choose(setting) {
		this.setting = setting;
		if (this.socket && this.socket.readyState === 1) {
			this.socket.send(JSON.stringify({ event: "setGlobalSettings", context: this.uuid, payload: { language: setting } }));
		}
		// "auto" rozstrzyga plugin (język paczki / systemu) – odpowie po zapisaniu ustawienia.
		if (setting !== "auto") this.apply(setting);
	},
};

function t(pl, en) {
	return I18N.lang === "pl" ? pl : en;
}

(() => {
	const style = document.createElement("style");
	// Panel jest niewidoczny, dopóki nie wiadomo, w jakim języku go pokazać.
	style.textContent = "body:not(.i18n-ready){visibility:hidden}.i18n-lang{margin-top:14px;border-top:1px solid #3d3d3d;padding-top:8px}";
	document.head.appendChild(style);

	window.addEventListener("DOMContentLoaded", () => {
		const row = document.createElement("div");
		row.className = "row i18n-lang";
		row.innerHTML =
			'<label for="i18n-language">Język / Language</label>' +
			'<select id="i18n-language"><option value="auto">Auto</option><option value="pl">Polski</option><option value="en">English</option></select>';
		document.body.appendChild(row);
		row.querySelector("select").addEventListener("change", (e) => I18N.choose(e.target.value));
		// Gdyby plugin nie odpowiedział (np. stara wersja) – język przeglądarki.
		setTimeout(() => {
			if (!I18N.lang) I18N.apply((navigator.language || "").startsWith("pl") ? "pl" : "en");
		}, 600);
	});
})();
