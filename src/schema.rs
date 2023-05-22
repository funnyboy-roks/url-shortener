diesel::table! {
    urls (slug) {
        slug -> Text,
        url -> Text,
        author_ip -> Text,
        usage_count -> Integer,
    }
}
