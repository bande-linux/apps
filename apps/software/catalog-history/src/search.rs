use bande_search::{Query, SUBSTRING, fold};

use crate::component::Component;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Field {
    Summary,
    Keyword,
    Name,
}

fn rank(query: &Query, app: &Component) -> Option<(Field, u32)> {
    let loose = |text: &str| query.score(&fold(text)).filter(|score| *score >= SUBSTRING);
    query
        .score(&fold(&app.name))
        .map(|score| (Field::Name, score))
        .or_else(|| {
            app.keywords
                .iter()
                .filter_map(|keyword| loose(keyword))
                .max()
                .map(|score| (Field::Keyword, score))
        })
        .or_else(|| loose(&app.summary).map(|score| (Field::Summary, score)))
}

pub fn search<'a>(apps: &'a [Component], text: &str) -> Vec<&'a Component> {
    let Some(query) = Query::new(text) else {
        return apps.iter().collect();
    };
    let mut ranked: Vec<_> = apps
        .iter()
        .filter_map(|app| rank(&query, app).map(|rank| (rank, app)))
        .collect();
    ranked.sort_by(|(left, a), (right, b)| right.cmp(left).then_with(|| a.name.cmp(&b.name)));
    ranked.into_iter().map(|(_, app)| app).collect()
}
