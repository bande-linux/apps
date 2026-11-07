use bande_media::{FailureKind, MediaInfo, Track};
use gstreamer as gst;

const MISSING_PLUGIN: &str = "missing-plugin";

fn track(stream: &gst::Stream) -> Option<Track> {
    let id = stream.stream_id()?.to_string();
    let tags = stream.tags();
    let text = |value: Option<String>| value.filter(|text| !text.is_empty());
    Some(Track {
        id,
        title: tags
            .as_ref()
            .and_then(|tags| text(tags.get::<gst::tags::Title>().map(|t| t.get().to_owned()))),
        language: tags.as_ref().and_then(|tags| {
            text(
                tags.get::<gst::tags::LanguageCode>()
                    .map(|t| t.get().to_owned()),
            )
        }),
    })
}

pub fn apply_collection(media: &mut MediaInfo, collection: &gst::StreamCollection) {
    let of_type = |kind: gst::StreamType| -> Vec<Track> {
        collection
            .iter()
            .filter(|stream| stream.stream_type().contains(kind))
            .filter_map(|stream| track(&stream))
            .collect()
    };
    media.audio_tracks = of_type(gst::StreamType::AUDIO);
    media.subtitle_tracks = of_type(gst::StreamType::TEXT);
    media.has_audio = !media.audio_tracks.is_empty();
}

pub fn apply_selection(media: &mut MediaInfo, selected: &[gst::Stream]) {
    let chosen = |kind: gst::StreamType| {
        selected
            .iter()
            .find(|stream| stream.stream_type().contains(kind))
            .and_then(|stream| stream.stream_id())
            .map(|id| id.to_string())
    };
    media.selected_audio = chosen(gst::StreamType::AUDIO);
    media.selected_subtitles = chosen(gst::StreamType::TEXT);
}

pub fn chosen_streams(
    collection: &gst::StreamCollection,
    audio: Option<&str>,
    subtitles: Option<&str>,
) -> Vec<String> {
    let streams: Vec<gst::Stream> = collection.iter().collect();
    let id = |stream: &gst::Stream| stream.stream_id().map(|id| id.to_string());
    let of_kind = |kind: gst::StreamType| {
        streams
            .iter()
            .filter(move |stream| stream.stream_type().contains(kind))
    };
    let mut chosen = Vec::new();
    chosen.extend(of_kind(gst::StreamType::VIDEO).next().and_then(id));
    let wanted_audio = of_kind(gst::StreamType::AUDIO)
        .find(|stream| audio.is_some_and(|wanted| id(stream).as_deref() == Some(wanted)))
        .or_else(|| of_kind(gst::StreamType::AUDIO).next());
    chosen.extend(wanted_audio.and_then(id));
    chosen.extend(
        of_kind(gst::StreamType::TEXT)
            .find(|stream| subtitles.is_some_and(|wanted| id(stream).as_deref() == Some(wanted)))
            .and_then(id),
    );
    chosen
}

pub fn apply_tags(media: &mut MediaInfo, tags: &gst::TagList) {
    let text = |value: Option<&str>| value.filter(|text| !text.is_empty()).map(str::to_owned);
    if let Some(title) = text(tags.get::<gst::tags::Title>().as_ref().map(|t| t.get())) {
        media.title = Some(title);
    }
    if let Some(artist) = text(tags.get::<gst::tags::Artist>().as_ref().map(|t| t.get())) {
        media.artist = Some(artist);
    }
    if let Some(album) = text(tags.get::<gst::tags::Album>().as_ref().map(|t| t.get())) {
        media.album = Some(album);
    }
}

pub fn cover_art(tags: &gst::TagList) -> Option<Vec<u8>> {
    let sample = tags
        .get::<gst::tags::Image>()
        .map(|image| image.get().clone())
        .or_else(|| {
            tags.get::<gst::tags::PreviewImage>()
                .map(|image| image.get().clone())
        })?;
    let map = sample.buffer()?.map_readable().ok()?;
    Some(map.as_slice().to_vec())
}

pub fn failure(error: gst::glib::Error, debug: Option<&str>) -> (FailureKind, String) {
    let kind = match (
        error.kind::<gst::ResourceError>(),
        error.kind::<gst::StreamError>(),
    ) {
        (Some(_), _) => FailureKind::Unreadable,
        (
            _,
            Some(
                gst::StreamError::TypeNotFound
                | gst::StreamError::CodecNotFound
                | gst::StreamError::WrongType
                | gst::StreamError::Decrypt,
            ),
        ) => FailureKind::Unsupported,
        _ => FailureKind::Failed,
    };
    let detail = match debug {
        Some(debug) => format!("{error} ({debug})"),
        None => error.to_string(),
    };
    (kind, detail)
}

pub fn missing_plugin(structure: Option<&gst::StructureRef>) -> Option<String> {
    let structure = structure.filter(|structure| structure.name() == MISSING_PLUGIN)?;
    structure.get::<String>("name").ok()
}
