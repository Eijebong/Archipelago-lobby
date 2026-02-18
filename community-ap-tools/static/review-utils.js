let _toastTimeout = null;
function showToast(msg, type = "error") {
    let el = document.getElementById("toast");
    if (!el) {
        el = document.createElement("div");
        el.id = "toast";
        document.body.appendChild(el);
    }
    el.className = `toast ${type}`;
    el.textContent = msg;
    el.classList.add("visible");
    clearTimeout(_toastTimeout);
    _toastTimeout = setTimeout(() => el.classList.remove("visible"), 4000);
}

function h(tag, attrs, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
        if (v == null || v === false) continue;
        if (k === "className") el.className = v;
        else if (k.startsWith("on") || k === "value" || k === "selected" || k === "disabled" || k === "checked") el[k] = v;
        else el.setAttribute(k, String(v));
    }
    for (const c of children) if (c != null && c !== false) el.append(c);
    return el;
}

function field(label, input) {
    return h("div", { className: "field" }, h("span", null, label), input);
}

function selectEl(className, options, selected) {
    return h("select", { className }, ...options.map(([val, text]) =>
        h("option", { value: val, selected: val === selected }, text)
    ));
}

function confirmDelete(name, callback) {
    const cancelBtn = h("button", { className: "small", onclick: () => dialog.remove() }, "Close");
    const deleteBtn = h("button", { className: "small danger", onclick: () => { dialog.remove(); callback(); } }, "Yes, delete it");
    const dialog = h("dialog", { className: "delete-popup" },
        h("span", { className: "popup-title" }, "Are you sure?"),
        h("div", { className: "popup-content" }, `Are you sure you want to delete "${name}"?`),
        h("div", { className: "popup-buttons" }, cancelBtn, deleteBtn),
    );
    dialog.onclick = (e) => { if (e.target === dialog) dialog.remove(); };
    document.body.appendChild(dialog);
    dialog.showModal();
}
