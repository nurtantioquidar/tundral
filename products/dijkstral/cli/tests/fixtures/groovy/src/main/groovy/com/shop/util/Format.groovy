package com.shop.util

class Format {
    static String title(String s) {
        return s ? s.capitalize() : s
    }

    static String slug(String s) {
        return s.toLowerCase().replaceAll(/\s+/, '-')
    }
}

class Banner {
    static void show(int n) {
        System.out.println("users: " + n)
    }
}
