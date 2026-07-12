use std::collections::HashMap;
use ebml_schema::{parse_xml, create_consts};

// create_consts!("../specs/ebml_matroska.xml");
// The macro works, but my IDE doesn't get it, so I just copy pasted the result into here
pub const ID_EBML: u64 = 0x1A45DFA3; pub const ID_EBMLVERSION: u64 = 0x4286; pub const ID_EBMLREAD_VERSION: u64 = 0x42F7; pub const ID_EBMLMAX_IDLENGTH: u64 = 0x42F2; pub const ID_EBMLMAX_SIZE_LENGTH: u64 = 0x42F3; pub const ID_DOCTYPE: u64 = 0x4282; pub const ID_DOCTYPE_VERSION: u64 = 0x4287; pub const ID_DOCTYPE_READ_VERSION: u64 = 0x4285; pub const ID_DOCTYPE_EXTENSION: u64 = 0x4281; pub const ID_DOCTYPE_EXTENSION_NAME: u64 = 0x4283; pub const ID_DOCTYPE_EXTENSION_VERSION: u64 = 0x4284; pub const ID_CRC32: u64 = 0xBF; pub const ID_VOID: u64 = 0xEC; pub const ID_SEGMENT : u64 = 408125543u64; pub const ID_SEEKHEAD : u64 = 290298740u64; pub const ID_SEEK : u64 = 19899u64; pub const ID_SEEKID : u64 = 21419u64; pub const ID_SEEKPOSITION : u64 = 21420u64; pub const ID_INFO : u64 = 357149030u64; pub const ID_SEGMENTUUID : u64 = 29604u64; pub const ID_SEGMENTFILENAME : u64 = 29572u64; pub const ID_PREVUUID : u64 = 3979555u64; pub const ID_PREVFILENAME : u64 = 3965867u64; pub const ID_NEXTUUID : u64 = 4110627u64; pub const ID_NEXTFILENAME : u64 = 4096955u64; pub const ID_SEGMENTFAMILY : u64 = 17476u64; pub const ID_CHAPTERTRANSLATE : u64 = 26916u64; pub const ID_CHAPTERTRANSLATEID : u64 = 27045u64; pub const ID_CHAPTERTRANSLATECODEC : u64 = 27071u64; pub const ID_CHAPTERTRANSLATEEDITIONUID : u64 = 27132u64; pub const ID_TIMESTAMPSCALE : u64 = 2807729u64; pub const ID_DURATION : u64 = 17545u64; pub const ID_DATEUTC : u64 = 17505u64; pub const ID_TITLE : u64 = 31657u64; pub const ID_MUXINGAPP : u64 = 19840u64; pub const ID_WRITINGAPP : u64 = 22337u64; pub const ID_CLUSTER : u64 = 524531317u64; pub const ID_TIMESTAMP : u64 = 231u64; pub const ID_SILENTTRACKS : u64 = 22612u64; pub const ID_SILENTTRACKNUMBER : u64 = 22743u64; pub const ID_POSITION : u64 = 167u64; pub const ID_PREVSIZE : u64 = 171u64; pub const ID_SIMPLEBLOCK : u64 = 163u64; pub const ID_BLOCKGROUP : u64 = 160u64; pub const ID_BLOCK : u64 = 161u64; pub const ID_BLOCKVIRTUAL : u64 = 162u64; pub const ID_BLOCKADDITIONS : u64 = 30113u64; pub const ID_BLOCKMORE : u64 = 166u64; pub const ID_BLOCKADDITIONAL : u64 = 165u64; pub const ID_BLOCKADDID : u64 = 238u64; pub const ID_BLOCKDURATION : u64 = 155u64; pub const ID_REFERENCEPRIORITY : u64 = 250u64; pub const ID_REFERENCEBLOCK : u64 = 251u64; pub const ID_REFERENCEVIRTUAL : u64 = 253u64; pub const ID_CODECSTATE : u64 = 164u64; pub const ID_DISCARDPADDING : u64 = 30114u64; pub const ID_SLICES : u64 = 142u64; pub const ID_TIMESLICE : u64 = 232u64; pub const ID_LACENUMBER : u64 = 204u64; pub const ID_FRAMENUMBER : u64 = 205u64; pub const ID_BLOCKADDITIONID : u64 = 203u64; pub const ID_DELAY : u64 = 206u64; pub const ID_SLICEDURATION : u64 = 207u64; pub const ID_REFERENCEFRAME : u64 = 200u64; pub const ID_REFERENCEOFFSET : u64 = 201u64; pub const ID_REFERENCETIMESTAMP : u64 = 202u64; pub const ID_ENCRYPTEDBLOCK : u64 = 175u64; pub const ID_TRACKS : u64 = 374648427u64; pub const ID_TRACKENTRY : u64 = 174u64; pub const ID_TRACKNUMBER : u64 = 215u64; pub const ID_TRACKUID : u64 = 29637u64; pub const ID_TRACKTYPE : u64 = 131u64; pub const ID_FLAGENABLED : u64 = 185u64; pub const ID_FLAGDEFAULT : u64 = 136u64; pub const ID_FLAGFORCED : u64 = 21930u64; pub const ID_FLAGHEARINGIMPAIRED : u64 = 21931u64; pub const ID_FLAGVISUALIMPAIRED : u64 = 21932u64; pub const ID_FLAGTEXTDESCRIPTIONS : u64 = 21933u64; pub const ID_FLAGORIGINAL : u64 = 21934u64; pub const ID_FLAGCOMMENTARY : u64 = 21935u64; pub const ID_FLAGLACING : u64 = 156u64; pub const ID_MINCACHE : u64 = 28135u64; pub const ID_MAXCACHE : u64 = 28152u64; pub const ID_DEFAULTDURATION : u64 = 2352003u64; pub const ID_DEFAULTDECODEDFIELDDURATION : u64 = 2313850u64; pub const ID_TRACKTIMESTAMPSCALE : u64 = 2306383u64; pub const ID_TRACKOFFSET : u64 = 21375u64; pub const ID_MAXBLOCKADDITIONID : u64 = 21998u64; pub const ID_BLOCKADDITIONMAPPING : u64 = 16868u64; pub const ID_BLOCKADDIDVALUE : u64 = 16880u64; pub const ID_BLOCKADDIDNAME : u64 = 16804u64; pub const ID_BLOCKADDIDTYPE : u64 = 16871u64; pub const ID_BLOCKADDIDEXTRADATA : u64 = 16877u64; pub const ID_NAME : u64 = 21358u64; pub const ID_LANGUAGE : u64 = 2274716u64; pub const ID_LANGUAGEBCP47 : u64 = 2274717u64; pub const ID_CODECID : u64 = 134u64; pub const ID_CODECPRIVATE : u64 = 25506u64; pub const ID_CODECNAME : u64 = 2459272u64; pub const ID_ATTACHMENTLINK : u64 = 29766u64; pub const ID_CODECSETTINGS : u64 = 3839639u64; pub const ID_CODECINFOURL : u64 = 3883072u64; pub const ID_CODECDOWNLOADURL : u64 = 2536000u64; pub const ID_CODECDECODEALL : u64 = 170u64; pub const ID_TRACKOVERLAY : u64 = 28587u64; pub const ID_CODECDELAY : u64 = 22186u64; pub const ID_SEEKPREROLL : u64 = 22203u64; pub const ID_TRACKTRANSLATE : u64 = 26148u64; pub const ID_TRACKTRANSLATETRACKID : u64 = 26277u64; pub const ID_TRACKTRANSLATECODEC : u64 = 26303u64; pub const ID_TRACKTRANSLATEEDITIONUID : u64 = 26364u64; pub const ID_VIDEO : u64 = 224u64; pub const ID_FLAGINTERLACED : u64 = 154u64; pub const ID_FIELDORDER : u64 = 157u64; pub const ID_STEREOMODE : u64 = 21432u64; pub const ID_ALPHAMODE : u64 = 21440u64; pub const ID_OLDSTEREOMODE : u64 = 21433u64; pub const ID_PIXELWIDTH : u64 = 176u64; pub const ID_PIXELHEIGHT : u64 = 186u64; pub const ID_PIXELCROPBOTTOM : u64 = 21674u64; pub const ID_PIXELCROPTOP : u64 = 21691u64; pub const ID_PIXELCROPLEFT : u64 = 21708u64; pub const ID_PIXELCROPRIGHT : u64 = 21725u64; pub const ID_DISPLAYWIDTH : u64 = 21680u64; pub const ID_DISPLAYHEIGHT : u64 = 21690u64; pub const ID_DISPLAYUNIT : u64 = 21682u64; pub const ID_ASPECTRATIOTYPE : u64 = 21683u64; pub const ID_UNCOMPRESSEDFOURCC : u64 = 3061028u64; pub const ID_GAMMAVALUE : u64 = 3126563u64; pub const ID_FRAMERATE : u64 = 2327523u64; pub const ID_COLOUR : u64 = 21936u64; pub const ID_MATRIXCOEFFICIENTS : u64 = 21937u64; pub const ID_BITSPERCHANNEL : u64 = 21938u64; pub const ID_CHROMASUBSAMPLINGHORZ : u64 = 21939u64; pub const ID_CHROMASUBSAMPLINGVERT : u64 = 21940u64; pub const ID_CBSUBSAMPLINGHORZ : u64 = 21941u64; pub const ID_CBSUBSAMPLINGVERT : u64 = 21942u64; pub const ID_CHROMASITINGHORZ : u64 = 21943u64; pub const ID_CHROMASITINGVERT : u64 = 21944u64; pub const ID_RANGE : u64 = 21945u64; pub const ID_TRANSFERCHARACTERISTICS : u64 = 21946u64; pub const ID_PRIMARIES : u64 = 21947u64; pub const ID_MAXCLL : u64 = 21948u64; pub const ID_MAXFALL : u64 = 21949u64; pub const ID_MASTERINGMETADATA : u64 = 21968u64; pub const ID_PRIMARYRCHROMATICITYX : u64 = 21969u64; pub const ID_PRIMARYRCHROMATICITYY : u64 = 21970u64; pub const ID_PRIMARYGCHROMATICITYX : u64 = 21971u64; pub const ID_PRIMARYGCHROMATICITYY : u64 = 21972u64; pub const ID_PRIMARYBCHROMATICITYX : u64 = 21973u64; pub const ID_PRIMARYBCHROMATICITYY : u64 = 21974u64; pub const ID_WHITEPOINTCHROMATICITYX : u64 = 21975u64; pub const ID_WHITEPOINTCHROMATICITYY : u64 = 21976u64; pub const ID_LUMINANCEMAX : u64 = 21977u64; pub const ID_LUMINANCEMIN : u64 = 21978u64; pub const ID_PROJECTION : u64 = 30320u64; pub const ID_PROJECTIONTYPE : u64 = 30321u64; pub const ID_PROJECTIONPRIVATE : u64 = 30322u64; pub const ID_PROJECTIONPOSEYAW : u64 = 30323u64; pub const ID_PROJECTIONPOSEPITCH : u64 = 30324u64; pub const ID_PROJECTIONPOSEROLL : u64 = 30325u64; pub const ID_AUDIO : u64 = 225u64; pub const ID_SAMPLINGFREQUENCY : u64 = 181u64; pub const ID_OUTPUTSAMPLINGFREQUENCY : u64 = 30901u64; pub const ID_CHANNELS : u64 = 159u64; pub const ID_CHANNELPOSITIONS : u64 = 32123u64; pub const ID_BITDEPTH : u64 = 25188u64; pub const ID_EMPHASIS : u64 = 21233u64; pub const ID_TRACKOPERATION : u64 = 226u64; pub const ID_TRACKCOMBINEPLANES : u64 = 227u64; pub const ID_TRACKPLANE : u64 = 228u64; pub const ID_TRACKPLANEUID : u64 = 229u64; pub const ID_TRACKPLANETYPE : u64 = 230u64; pub const ID_TRACKJOINBLOCKS : u64 = 233u64; pub const ID_TRACKJOINUID : u64 = 237u64; pub const ID_TRICKTRACKUID : u64 = 192u64; pub const ID_TRICKTRACKSEGMENTUID : u64 = 193u64; pub const ID_TRICKTRACKFLAG : u64 = 198u64; pub const ID_TRICKMASTERTRACKUID : u64 = 199u64; pub const ID_TRICKMASTERTRACKSEGMENTUID : u64 = 196u64; pub const ID_CONTENTENCODINGS : u64 = 28032u64; pub const ID_CONTENTENCODING : u64 = 25152u64; pub const ID_CONTENTENCODINGORDER : u64 = 20529u64; pub const ID_CONTENTENCODINGSCOPE : u64 = 20530u64; pub const ID_CONTENTENCODINGTYPE : u64 = 20531u64; pub const ID_CONTENTCOMPRESSION : u64 = 20532u64; pub const ID_CONTENTCOMPALGO : u64 = 16980u64; pub const ID_CONTENTCOMPSETTINGS : u64 = 16981u64; pub const ID_CONTENTENCRYPTION : u64 = 20533u64; pub const ID_CONTENTENCALGO : u64 = 18401u64; pub const ID_CONTENTENCKEYID : u64 = 18402u64; pub const ID_CONTENTENCAESSETTINGS : u64 = 18407u64; pub const ID_AESSETTINGSCIPHERMODE : u64 = 18408u64; pub const ID_CONTENTSIGNATURE : u64 = 18403u64; pub const ID_CONTENTSIGKEYID : u64 = 18404u64; pub const ID_CONTENTSIGALGO : u64 = 18405u64; pub const ID_CONTENTSIGHASHALGO : u64 = 18406u64; pub const ID_CUES : u64 = 475249515u64; pub const ID_CUEPOINT : u64 = 187u64; pub const ID_CUETIME : u64 = 179u64; pub const ID_CUETRACKPOSITIONS : u64 = 183u64; pub const ID_CUETRACK : u64 = 247u64; pub const ID_CUECLUSTERPOSITION : u64 = 241u64; pub const ID_CUERELATIVEPOSITION : u64 = 240u64; pub const ID_CUEDURATION : u64 = 178u64; pub const ID_CUEBLOCKNUMBER : u64 = 21368u64; pub const ID_CUECODECSTATE : u64 = 234u64; pub const ID_CUEREFERENCE : u64 = 219u64; pub const ID_CUEREFTIME : u64 = 150u64; pub const ID_CUEREFCLUSTER : u64 = 151u64; pub const ID_CUEREFNUMBER : u64 = 21343u64; pub const ID_CUEREFCODECSTATE : u64 = 235u64; pub const ID_ATTACHMENTS : u64 = 423732329u64; pub const ID_ATTACHEDFILE : u64 = 24999u64; pub const ID_FILEDESCRIPTION : u64 = 18046u64; pub const ID_FILENAME : u64 = 18030u64; pub const ID_FILEMEDIATYPE : u64 = 18016u64; pub const ID_FILEDATA : u64 = 18012u64; pub const ID_FILEUID : u64 = 18094u64; pub const ID_FILEREFERRAL : u64 = 18037u64; pub const ID_FILEUSEDSTARTTIME : u64 = 18017u64; pub const ID_FILEUSEDENDTIME : u64 = 18018u64; pub const ID_CHAPTERS : u64 = 272869232u64; pub const ID_EDITIONENTRY : u64 = 17849u64; pub const ID_EDITIONUID : u64 = 17852u64; pub const ID_EDITIONFLAGHIDDEN : u64 = 17853u64; pub const ID_EDITIONFLAGDEFAULT : u64 = 17883u64; pub const ID_EDITIONFLAGORDERED : u64 = 17885u64; pub const ID_EDITIONDISPLAY : u64 = 17696u64; pub const ID_EDITIONSTRING : u64 = 17697u64; pub const ID_EDITIONLANGUAGEIETF : u64 = 17892u64; pub const ID_CHAPTERATOM : u64 = 182u64; pub const ID_CHAPTERUID : u64 = 29636u64; pub const ID_CHAPTERSTRINGUID : u64 = 22100u64; pub const ID_CHAPTERTIMESTART : u64 = 145u64; pub const ID_CHAPTERTIMEEND : u64 = 146u64; pub const ID_CHAPTERFLAGHIDDEN : u64 = 152u64; pub const ID_CHAPTERFLAGENABLED : u64 = 17816u64; pub const ID_CHAPTERSEGMENTUUID : u64 = 28263u64; pub const ID_CHAPTERSKIPTYPE : u64 = 17800u64; pub const ID_CHAPTERSEGMENTEDITIONUID : u64 = 28348u64; pub const ID_CHAPTERPHYSICALEQUIV : u64 = 25539u64; pub const ID_CHAPTERTRACK : u64 = 143u64; pub const ID_CHAPTERTRACKUID : u64 = 137u64; pub const ID_CHAPTERDISPLAY : u64 = 128u64; pub const ID_CHAPSTRING : u64 = 133u64; pub const ID_CHAPLANGUAGE : u64 = 17276u64; pub const ID_CHAPLANGUAGEBCP47 : u64 = 17277u64; pub const ID_CHAPCOUNTRY : u64 = 17278u64; pub const ID_CHAPPROCESS : u64 = 26948u64; pub const ID_CHAPPROCESSCODECID : u64 = 26965u64; pub const ID_CHAPPROCESSPRIVATE : u64 = 17677u64; pub const ID_CHAPPROCESSCOMMAND : u64 = 26897u64; pub const ID_CHAPPROCESSTIME : u64 = 26914u64; pub const ID_CHAPPROCESSDATA : u64 = 26931u64; pub const ID_TAGS : u64 = 307544935u64; pub const ID_TAG : u64 = 29555u64; pub const ID_TARGETS : u64 = 25536u64; pub const ID_TARGETTYPEVALUE : u64 = 26826u64; pub const ID_TARGETTYPE : u64 = 25546u64; pub const ID_TAGTRACKUID : u64 = 25541u64; pub const ID_TAGEDITIONUID : u64 = 25545u64; pub const ID_TAGCHAPTERUID : u64 = 25540u64; pub const ID_TAGATTACHMENTUID : u64 = 25542u64; pub const ID_TAGBLOCKADDIDVALUE : u64 = 25543u64; pub const ID_SIMPLETAG : u64 = 26568u64; pub const ID_TAGNAME : u64 = 17827u64; pub const ID_TAGLANGUAGE : u64 = 17530u64; pub const ID_TAGLANGUAGEBCP47 : u64 = 17531u64; pub const ID_TAGDEFAULT : u64 = 17540u64; pub const ID_TAGDEFAULTBOGUS : u64 = 17588u64; pub const ID_TAGSTRING : u64 = 17543u64; pub const ID_TAGBINARY : u64 = 17541u64;

#[derive(Debug, Clone, PartialEq)]
pub enum EbmlType {
    SignedInteger,
    UnsignedInteger,
    Float,
    String,
    UTF8,
    Date,
    Master,
    Binary,
    Void,
    Unsupported,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Element {
    pub name: String,
    pub path: String,
    pub id: u64,
    pub ebml_type: EbmlType,
    pub default: Option<String>,
    pub minOccurs: Option<i32>,
    pub maxOccurs: Option<i32>,
}

pub fn get_schema() -> Vec<Element> {
    parse_xml!("../specs/ebml_matroska.xml")
}

pub fn ebml_base_id_type_map() -> HashMap<u64, EbmlType> {
    let mut map: HashMap<u64, EbmlType> = HashMap::new();

    map.insert(ID_EBML, EbmlType::Master);
    map.insert(ID_EBMLVERSION, EbmlType::UnsignedInteger);
    map.insert(ID_EBMLREAD_VERSION, EbmlType::UnsignedInteger);
    map.insert(ID_EBMLMAX_IDLENGTH, EbmlType::UnsignedInteger);
    map.insert(ID_EBMLMAX_SIZE_LENGTH, EbmlType::UnsignedInteger);
    map.insert(ID_DOCTYPE, EbmlType::String);
    map.insert(ID_DOCTYPE_VERSION, EbmlType::UnsignedInteger);
    map.insert(ID_DOCTYPE_READ_VERSION, EbmlType::UnsignedInteger);

    map.insert(ID_DOCTYPE_EXTENSION, EbmlType::Master);
    map.insert(ID_DOCTYPE_EXTENSION_NAME, EbmlType::String);
    map.insert(ID_DOCTYPE_EXTENSION_VERSION, EbmlType::UnsignedInteger);

    map.insert(ID_CRC32, EbmlType::Binary);
    map.insert(ID_VOID, EbmlType::Void);

    map
}

pub fn element_id_type_map() -> HashMap<u64, EbmlType> {
    let schema = get_schema();
    let mut map: HashMap<u64, EbmlType> = ebml_base_id_type_map();

    for element in schema.iter() {
        println!("{:#X} ({}) -> {:?}", element.id, element.name, element.ebml_type);
        map.insert(element.id, element.ebml_type.clone());
    }

    map
}

pub fn element_id_element_map() -> HashMap<u64, Element> {
    let schema = get_schema();
    let mut map: HashMap<u64, Element> = HashMap::new();

    for element in schema.iter() {
        map.insert(element.id, element.clone());
    }

    map
}