package com.shop.model

/**
 * Root of every model.
 */
abstract class Base implements Named {
    // NOTE: ids are assigned by the repository.
    Long id

    String describe() {
        return "${getClass().simpleName}#${id}"
    }
}

interface Named {
    String getName()
}
