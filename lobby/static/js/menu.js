class Menu {
    constructor(anchor) {
        this.opened = false;
        this.items = [];
        this.elmt = null;
        this.anchor = anchor;
        this.resizeHandler = this.moveToAnchor.bind(this)
        this.clickHandler = this.handleClick.bind(this)
        this.closeHandler = this.close.bind(this)
    }

    build() {
        const elmt = document.createElement("ul")
        elmt.classList = "context-menu"
        elmt.style.visibility = "hidden";
        elmt.style.display = "block";

        for (const item of this.items) {
            const itemElmt = item.build();

            elmt.appendChild(itemElmt)
        }

        this.elmt = elmt;
        this.anchor.appendChild(this.elmt)
        this.moveToAnchor()
    }

    trigger() {
        if (this.opened) {
            this.close()
            return;
        }
        this.open()
    }

    open() {
        this.moveToAnchor()
        this.elmt.style.visibility = "visible";
        this.opened = true;

        addEventListener("resize", this.resizeHandler)
        addEventListener("mousedown", this.clickHandler)
        addEventListener("wheel", this.closeHandler)
    }

    close() {
        this.elmt.style.visibility = "hidden";
        this.opened = false;

        removeEventListener("resize", this.resizeHandler)
        removeEventListener("mousedown", this.clickHandler)
        removeEventListener("wheel", this.closeHandler)
    }

    moveToAnchor() {
        const anchor_bb = this.anchor.getBoundingClientRect();
        const element_bb = this.elmt.getBoundingClientRect();
        // If the menu doesn't fit in the page, offset it to the left
        if (anchor_bb.left + element_bb.width > window.innerWidth) {
            this.elmt.style.left = anchor_bb.right - element_bb.width + "px"
        } else {
            this.elmt.style.left = anchor_bb.left + "px"
        }

        if (anchor_bb.top + element_bb.height > window.innerHeight) {
            this.elmt.style.top = (anchor_bb.top - element_bb.height + window.scrollY) + "px";
        } else {
            this.elmt.style.top = (anchor_bb.bottom + window.scrollY) + "px"
        }

        this.elmt.style["min-width"] = anchor_bb.width + "px"
    }

    handleClick(event) {
        if (!this.anchor.contains(event.target)) {
            this.close()
        }
    }
}

class MenuItem {
    constructor(text, callback) {
        this.text = text
        this.callback = callback
    }

    build() {
        const elmt = document.createElement("li");
        elmt.innerText = this.text;
        elmt.addEventListener("click", this.callback)

        return elmt
    }
}

