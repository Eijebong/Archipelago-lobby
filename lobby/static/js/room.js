function openConfirmationPopup(resourceName, resourceType, callback) {
    const popup = document.createElement("dialog");
    popup.classList = "delete-popup";
    popup.onclick = (event) => { event.target == popup && popup.close(); return true; }

    const popupTitle = document.createElement("span");
    popupTitle.classList = "popup-title"
    popupTitle.innerText = "Are you sure?"
    popup.appendChild(popupTitle)

    const popupContent = document.createElement("div");
    popupContent.classList = "popup-content"
    popupContent.textContent = "Are you sure you want to delete the " + resourceType + " " + resourceName + "?";
    popup.appendChild(popupContent);

    const buttonContainer = document.createElement("div");
    buttonContainer.classList = "button-container";

    const deleteButton = document.createElement("a");
    deleteButton.innerText = "Yes, delete it";
    deleteButton.classList = "cancel-button";
    deleteButton.onclick = () => { callback(); }

    const closeButton = document.createElement("button");
    closeButton.innerText = "Close";
    closeButton.onclick = () => { popup.close(); };
    popup.onclose = () => body.removeChild(popup);

    buttonContainer.appendChild(closeButton);
    buttonContainer.appendChild(deleteButton);
    popup.appendChild(buttonContainer);

    const body = document.getElementById("main");
    body.append(popup);

    popup.showModal()
}

const deletableItems = document.querySelectorAll('[data-confirm-del]')
for(const item of deletableItems) {
    const resourceType = item.dataset.resourceType || "unknown"
    const resourceName = item.dataset.resourceName || "unknown"

    item.addEventListener('click', (event) => {
        if (event.cancelable) {
            event.preventDefault();
            openConfirmationPopup(resourceName, resourceType, () => {
                location.href = item.href;
            });
        }

    });
}


const multiselects = document.getElementsByClassName("multiselect")
for(const multiselect of multiselects) {
    const chooser = document.createElement("a")
    const menu = new Menu(chooser)
    chooser.className = "validation-button"
    chooser.innerText = "v";
    let first = true
    for (const item of multiselect.children) {
        menu.items.push(new MenuItem(item.innerText, (event) => {
            for (const item of multiselect.children) {
                if (item.innerText == chooser.innerText) {
                    continue
                }
                if (event.target.innerText != item.innerText) {
                    item.style.display = 'none';
                } else {
                    item.style.display = 'inline';
                }
            }
        }))
        if (first) {
            first = false
            continue
        }
        item.style.display = 'none'
    }
    menu.build();
    chooser.addEventListener('click', () => {
        menu.trigger()
    });
    multiselect.appendChild(chooser);
}
