package com.shop.service;

import java.lang.annotation.Retention;
import java.lang.annotation.RetentionPolicy;

/** Marks methods whose calls are audited. */
@Retention(RetentionPolicy.RUNTIME)
public @interface Audited {
    String value() default "";
}
