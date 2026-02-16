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

function esc(s) {
    if (s == null) return '';
    const d = document.createElement('div');
    d.textContent = String(s);
    return d.innerHTML.replace(/"/g, '&quot;').replace(/'/g, '&#39;');
}

function confirmDelete(name, callback) {
    const dialog = document.createElement("dialog");
    dialog.className = "delete-popup";
    dialog.onclick = (e) => { if (e.target === dialog) dialog.remove(); };
    dialog.innerHTML = `
        <span class="popup-title">Are you sure?</span>
        <div class="popup-content"></div>
        <div class="popup-buttons">
            <button class="small" id="confirm-cancel">Close</button>
            <button class="small danger" id="confirm-delete">Yes, delete it</button>
        </div>
    `;
    dialog.querySelector(".popup-content").textContent = `Are you sure you want to delete "${name}"?`;
    dialog.querySelector("#confirm-cancel").onclick = () => dialog.remove();
    dialog.querySelector("#confirm-delete").onclick = () => { dialog.remove(); callback(); };
    document.body.appendChild(dialog);
    dialog.showModal();
}
