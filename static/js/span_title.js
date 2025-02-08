const spanElements = document.getElementsByTagName("span");
var hasTitleShown = false;

for (const span of spanElements) {
    const hasTitle = span.title !== undefined && span.title !== "";

    if (!hasTitle) {
        continue;
    }

    const parent = span.parentElement;

    parent.addEventListener('mouseover', () => {
        if (hasTitleShown || span.dataset.forcedOn === "true") {
            return
        }

        hasTitleShown = true;
        const titleElem = createTitleSpan(span)

        parent.addEventListener('mouseleave', () => {
            hasTitleShown = false;
            parent.removeChild(titleElem);
        }, {once: true});
    });


}

function createTitleSpan(elem) {
    const parent = elem.parentElement;
    const titleElem = document.createElement("span");
    titleElem.innerHTML = " (" + elem.title + ")";
    titleElem.classList = "span-title"
    titleElem.style.display = 'inline';
    if (elem.nextSibling) {
        parent.insertBefore(titleElem, elem.nextSibling);
    } else {
        parent.appendChild(titleElem);
    }

    return titleElem
}
