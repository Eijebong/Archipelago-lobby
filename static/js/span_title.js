const spanElements = document.getElementsByTagName("span");
var hasTitleShown = false;

for (const span of spanElements) {
    const hasTitle = span.title !== undefined && span.title !== "";

    if (!hasTitle) {
        continue;
    }

    const parent = span.parentElement;

    parent.addEventListener('mouseover', () => {
        if (hasTitleShown) {
            return
        }

        hasTitleShown = true;
        const titleElem = document.createElement("span");
        titleElem.innerHTML = " (" + span.title + ")";
        titleElem.classList = "span-title"
        titleElem.style.display = 'inline';
        if (span.nextSibling) {
            parent.insertBefore(titleElem, span.nextSibling);
        } else {
            parent.appendChild(titleElem);
        }

        parent.addEventListener('mouseleave', () => {
            hasTitleShown = false;
            parent.removeChild(titleElem);
        }, {once: true});
    });


}

