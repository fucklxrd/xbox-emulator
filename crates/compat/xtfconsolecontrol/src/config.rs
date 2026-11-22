use crate::op::*;
use std::sync::OnceLock;
use xtfmsg::chk::Activity;
use xtfmsg::com::{array, json, Com, Obj};
use xtfmsg::msg::{deserialize_body, serialize_end_body};
use xtfmsg::sys::*;
use xtfmsg::xbtp;

pub const CONFIG_CPP: &str = "xbox\\devex\\xtf\\dll\\console\\config.cpp";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Setting {
    pub name: &'static str,
    pub category: &'static str,
    pub reboot_required: bool,
    pub hidden: bool,
    pub range: Option<&'static [&'static str]>,
}

const fn s(name: &'static str, category: &'static str, flags: u16, range: Option<&'static [&'static str]>) -> Setting {
    Setting { name, category, reboot_required: flags & 1 != 0, hidden: flags & 0x100 != 0, range }
}

const TRUE_FALSE: &[&str] = &["True", "False"];
const ON_OFF: &[&str] = &["On", "Off"];
const LOG_LEVELS: &[&str] = &["Quiet", "Detailed", "Verbose"];

pub const PREFERRED_LANGUAGES: &[&str] = &[
    "ar-SA", "ar-AE;ar-SA", "cs-CZ", "da-DK", "de-DE", "de-AT;de-DE", "de-CH;de-DE", "el-GR", "en-GB", "en-AE;en-GB", "en-AU;en-GB", "en-CA;en-GB", "en-CZ;en-GB", "en-GR;en-GB", "en-HK;en-GB", "en-HU;en-GB", "en-IE;en-GB", "en-IL;en-GB", "en-IN;en-GB", "en-NZ;en-GB", "en-SA;en-GB", "en-SG;en-GB", "en-SK;en-GB", "en-ZA;en-GB", "en-US", "es-ES", "es-MX", "es-AR;es-MX", "es-CL;es-MX", "es-CO;es-MX", "fi-FI", "fr-FR", "fr-BE;fr-FR", "fr-CA;fr-FR", "fr-CH;fr-FR", "he-IL", "hu-HU", "it-IT", "ja-JP", "ko-KR", "nb-NO", "nl-NL", "nl-BE;nl-NL", "pl-PL", "pt-BR", "pt-PT;pt-BR", "ru-RU", "sk-SK", "sv-SE", "tr-TR", "zh-CN", "zh-SG;zh-CN", "zh-TW", "zh-HK;zh-TW",
];

pub static SETTINGS: [Setting; 78] = [
    s("AudioBitstreamFormat", "Audio", 0, None),
    s("HDMIAudio", "Audio", 0, None),
    s("HeadsetFormat", "Audio", 0, Some(&["Stereo", "WindowsSonicForHeadphones", "DolbyAtmosForHeadphones", "DtsHeadphoneX"])),
    s("IsHDAudioMutedWithHeadset", "Audio", 0, None),
    s("OpticalAudio", "Audio", 0, None),
    s("UseHdmiOrOpticalAudioHeadset", "Audio", 0, None),
    s("consolemode", "Debug", 1, Some(&["Default", "XboxOne", "XboxOneS", "XboxOneXRetail", "XboxOneXDevkit", "XboxOneXDevkit44", "Lockhart", "Anaconda", "GameStreaming", "LockhartProfiling", "AnacondaProfiling", "Scarlett40GB"])),
    s("CrashDumpType", "Debug", 0, Some(&["None", "Triage", "Mini", "Heap"])),
    s("CrashDumpUpload", "Debug", 0, Some(TRUE_FALSE)),
    s("debugnic", "Debug", 1, Some(ON_OFF)),
    s("EnableEventLimitToast", "Debug", 0, None),
    s("EnableKernelDebugging", "Debug", 1, None),
    s("EnableVideoCrashdumps", "Debug", 0, None),
    s("eragraphicsdrivermode", "Debug", 0, Some(&["TitleConfigured", "Retail", "Instrumented", "Validated"])),
    s("extratitlememory", "Debug", 1, None),
    s("FastIterationMode", "Debug", 0x100, Some(ON_OFF)),
    s("LogLevel", "Debug", 0x100, Some(LOG_LEVELS)),
    s("pixfasttransportmode", "Debug", 0x100, Some(ON_OFF)),
    s("profilingmode", "Debug", 1, None),
    s("TitlePerformanceOverlay", "Debug", 0, None),
    s("toolingmemoryoverflow", "Debug", 0, Some(TRUE_FALSE)),
    s("ToolsLogLevel", "Debug", 0, Some(LOG_LEVELS)),
    s("allowhdr", "Display", 0, Some(TRUE_FALSE)),
    s("allow4k", "Display", 0, Some(TRUE_FALSE)),
    s("allowvrr", "Display", 0, Some(TRUE_FALSE)),
    s("allowycc", "Display", 0, Some(TRUE_FALSE)),
    s("colordepth", "Display", 0, None),
    s("colorspace", "Display", 0, None),
    s("displayconnection", "Display", 0, Some(&["Auto", "DVI", "HDMI", "HDMI2"])),
    s("displayresolution", "Display", 0, None),
    s("GameStreamingServerLocation", "GameStreaming", 0, None),
    s("debugmemorymode", "Legacy", 1, None),
    s("devkitAllowACG", "Legacy", 1, None),
    s("EnableKinectStudio", "Legacy", 0x101, None),
    s("KinectSensorDisabled", "Legacy", 0, Some(TRUE_FALSE)),
    s("legacygpuperformancemode", "Legacy", 1, Some(TRUE_FALSE)),
    s("simulateversionswitch", "Legacy", 1, None),
    s("sragraphicsdrivermode", "Legacy", 0x101, Some(&["Retail", "Instrumented", "Validated"])),
    s("titlecompatoverride", "Legacy", 0, Some(TRUE_FALSE)),
    s("disableforceguest", "Network", 0x101, Some(TRUE_FALSE)),
    s("dnsserver", "Network", 1, None),
    s("hostname", "Network", 1, None),
    s("httpproxyhost", "Network", 1, None),
    s("ipaddress", "Network", 1, None),
    s("macaddress", "Network", 1, None),
    s("networkaddressmode", "Network", 1, None),
    s("networkinitdelayinseconds", "Network", 0, None),
    s("networktype", "Network", 1, None),
    s("desiredpreferredlocaludpmultiplayerport", "Network", 0, None),
    s("currentpreferredlocaludpmultiplayerportstate", "Network", 0, None),
    s("wirelessradiosettings", "Network", 1, None),
    s("AlwaysOn", "Power", 1, None),
    s("AutoBoot", "Power", 1, None),
    s("dimtimeout", "Power", 1, None),
    s("idleshutdowntimeout", "Power", 0, None),
    s("powermode", "Power", 0, None),
    s("DefaultStorageDevice", "Storage", 0, Some(&["Retail", "Development", "Ext1", "Ext2", "Ext3", "Ext4", "Ext5", "Ext6", "Ext7"])),
    s("EnableInternalSSD", "Storage", 0x101, Some(TRUE_FALSE)),
    s("EnableUnifiedScratch", "Storage", 0x100, Some(TRUE_FALSE)),
    s("EnableRunFromPcDataCache", "Storage", 1, Some(TRUE_FALSE)),
    s("RunFromPcDataCacheSizeInGB", "Storage", 1, None),
    s("AccessKey", "System", 1, None),
    s("accessoryflags", "System", 0x101, None),
    s("allowxboxappconnections", "System", 0, Some(TRUE_FALSE)),
    s("consoleType", "System", 0, None),
    s("disableselectivesuspend", "System", 1, None),
    s("environment", "System", 0x101, None),
    s("geographicregion", "System", 1, None),
    s("oobecompleted", "System", 0x101, None),
    s("preferredlanguages", "System", 1, Some(PREFERRED_LANGUAGES)),
    s("sandboxid", "System", 1, None),
    s("timezone", "System", 0, None),
    s("VideoServer", "System", 0x100, Some(ON_OFF)),
    s("wdpenabled", "System", 0, Some(TRUE_FALSE)),
    s("osupdatepolicy", "Update", 0, Some(&["NoUpdate", "Current", "Preview"])),
    s("DefaultUser", "User", 0, None),
    s("DefaultUserPairing", "User", 1, None),
    s("HeadlessXUser", "User", 0, None),
];

pub fn wcsicmp(a: &str, b: &str) -> std::cmp::Ordering {
    let la = a.encode_utf16().map(|c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c });
    let lb = b.encode_utf16().map(|c| if (b'A' as u16..=b'Z' as u16).contains(&c) { c + 32 } else { c });
    la.cmp(lb)
}

pub fn sorted_list() -> &'static [Setting] {
    static L: OnceLock<Vec<Setting>> = OnceLock::new();
    L.get_or_init(|| {
        let mut v = SETTINGS.to_vec();
        v.sort_by(|a, b| wcsicmp(a.category, b.category).then_with(|| wcsicmp(a.name, b.name)));
        v
    })
}

fn tf(b: bool) -> &'static str {
    if b {
        "true"
    } else {
        "false"
    }
}

pub unsafe extern "system" fn get_config_value_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GetConfigValueHandler\0");
    let mut op = Op::new(CONFIG_CPP, "GetConfigValueHandler", reply);
    let mut req = Com::null();
    let mut body = Com::null();
    let mut key = Bstr::null();
    let value = Bstr::null();
    'done: {
        let hr = deserialize_body(request as Obj, req.put());
        if hr < 0 {
            op.fail(hr, 1451, "XtfDeserializeJsonBody(pReceived, &reqBody)");
            break 'done;
        }
        let hr = get_string(req.get(), "ConfigKey", &mut key);
        if hr < 0 {
            op.fail(hr, 1452, "reqBody->GetString(ConfigKeyPropertyName, &strKey)");
            break 'done;
        }
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 1454, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = set_string(body.get(), "ConfigValue", value.ptr());
        if hr < 0 {
            op.fail(hr, 1458, "replyBody->SetString(ConfigValuePropertyName, strValue)");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 1459, "XtfSerializeJsonBody(replyBody, pReply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce(key.ptr(), core::ptr::null());
    }
    op.complete(reply, body.get());
    op.finish(activity)
}

pub unsafe extern "system" fn set_config_value_handler(_session: usize, request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: SetConfigValueHandler\0");
    let mut op = Op::new(CONFIG_CPP, "SetConfigValueHandler", reply);
    let mut req = Com::null();
    let mut body = Com::null();
    let mut key = Bstr::null();
    let mut value = Bstr::null();
    'done: {
        let hr = deserialize_body(request as Obj, req.put());
        if hr < 0 {
            op.fail(hr, 1487, "XtfDeserializeJsonBody(pReceived, &reqBody)");
            break 'done;
        }
        let hr = get_string(req.get(), "ConfigKey", &mut key);
        if hr < 0 {
            op.fail(hr, 1488, "reqBody->GetString(ConfigKeyPropertyName, &strKey)");
            break 'done;
        }
        let hr = get_string(req.get(), "ConfigValue", &mut value);
        if hr < 0 {
            op.fail(hr, 1489, "reqBody->GetString(ConfigValuePropertyName, &strValue)");
            break 'done;
        }
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 1490, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
    }
    op.xce(key.ptr(), value.ptr());
    op.complete(reply, body.get());
    op.finish(activity)
}

pub unsafe extern "system" fn get_all_settings_handler(_session: usize, _request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GetAllSettingsHandler\0");
    let mut op = Op::new(CONFIG_CPP, "GetAllSettingsHandler", reply);
    let mut body = Com::null();
    let mut settings_body = Com::null();
    let mut settings = Com::null();
    let value = Bstr::null();
    'done: {
        let list = sorted_list();
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 1527, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = xbtp::create_json_object(settings_body.put());
        if hr < 0 {
            op.fail(hr, 1528, "XbtpCreateJsonObject(&settingsBody)");
            break 'done;
        }
        let hr = xbtp::create_json_array(settings.put());
        if hr < 0 {
            op.fail(hr, 1529, "XbtpCreateJsonArray(&settingsArray)");
            break 'done;
        }
        for st in list {
            let mut setting = Com::null();
            let hr = xbtp::create_json_object(setting.put());
            if hr < 0 {
                op.fail(hr, 1535, "XbtpCreateJsonObject(&setting)");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "SettingName", st.name);
            if hr < 0 {
                op.fail(hr, 1537, "setting->SetString(SettingNameProperty, ConsoleConfig::g_sortedList[i].pszName)");
                break 'done;
            }
            let hr = set_string(setting.get(), "SettingValue", value.ptr());
            if hr < 0 {
                op.fail(hr, 1538, "setting->SetString(SettingValueProperty, strValue)");
                break 'done;
            }
            let hr = array::append_object(settings.get(), setting.get());
            if hr < 0 {
                op.fail(hr, 1539, "settingsArray->AppendObject(setting)");
                break 'done;
            }
        }
        let n = w("AllSettings");
        let hr = json::set_array(settings_body.get(), n.as_ptr(), settings.get());
        if hr < 0 {
            op.fail(hr, 1542, "settingsBody->SetArray(AllSettingsArrayProperty, settingsArray)");
            break 'done;
        }
        let n = w("SettingsArrayObject");
        let hr = json::set_object(body.get(), n.as_ptr(), settings_body.get());
        if hr < 0 {
            op.fail(hr, 1543, "replyBody->SetObject(AllSettingsArrayObjectProperty, settingsBody)");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 1544, "XtfSerializeJsonBody(replyBody, pReply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    op.complete(reply, body.get());
    op.finish(activity)
}

pub fn range_text(range: &[&str]) -> String {
    range.join(";")
}

pub unsafe extern "system" fn get_settings_metadata_handler(_session: usize, _request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GetSettingsMetadataHandler\0");
    let mut op = Op::new(CONFIG_CPP, "GetSettingsMetadataHandler", reply);
    let mut body = Com::null();
    let mut settings_body = Com::null();
    let mut settings = Com::null();
    let value = Bstr::null();
    'done: {
        let list = sorted_list();
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 1582, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = xbtp::create_json_object(settings_body.put());
        if hr < 0 {
            op.fail(hr, 1583, "XbtpCreateJsonObject(&settingsBody)");
            break 'done;
        }
        let hr = xbtp::create_json_array(settings.put());
        if hr < 0 {
            op.fail(hr, 1584, "XbtpCreateJsonArray(&settingsArray)");
            break 'done;
        }
        for st in list {
            let mut setting = Com::null();
            let mut md = Com::null();
            let hr = xbtp::create_json_object(setting.put());
            if hr < 0 {
                op.fail(hr, 1592, "XbtpCreateJsonObject(&setting)");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "SettingName", st.name);
            if hr < 0 {
                op.fail(hr, 1594, "setting->SetString(SettingNameProperty, settingMetadata.pszName)");
                break 'done;
            }
            let hr = set_string(setting.get(), "SettingValue", value.ptr());
            if hr < 0 {
                op.fail(hr, 1595, "setting->SetString(SettingValueProperty, strValue)");
                break 'done;
            }
            let hr = xbtp::create_json_array(md.put());
            if hr < 0 {
                op.fail(hr, 1597, "XbtpCreateJsonArray(&metadataArray)");
                break 'done;
            }
            let mut entries: Vec<(&str, Option<String>, [u32; 4], [&str; 4])> = vec![
                ("Category", Some(st.category.to_string()), [1600, 1601, 1602, 1603], [
                    "XbtpCreateJsonObject(&metadata)",
                    "metadata->SetString(SettingMetadataNameProperty, ConsoleConfig::g_pszMetadataCategory)",
                    "metadata->SetString(SettingMetadataValueProperty, settingMetadata.pszCategory)",
                    "metadataArray->AppendObject(metadata)",
                ]),
                ("RebootRequired", Some(tf(st.reboot_required).to_string()), [1606, 1607, 1610, 1611], [
                    "XbtpCreateJsonObject(&metadata)",
                    "metadata->SetString(SettingMetadataNameProperty, ConsoleConfig::g_pszMetadataRebootRequired)",
                    "metadata->SetString( SettingMetadataValueProperty, (settingMetadata.fRebootRequired == false ? L\"false\" : L\"true\"))",
                    "metadataArray->AppendObject(metadata)",
                ]),
                ("Hidden", Some(tf(st.hidden).to_string()), [1614, 1615, 1618, 1619], [
                    "XbtpCreateJsonObject(&metadata)",
                    "metadata->SetString(SettingMetadataNameProperty, ConsoleConfig::g_pszMetadataHidden)",
                    "metadata->SetString( SettingMetadataValueProperty, (settingMetadata.fHidden == false ? L\"false\" : L\"true\"))",
                    "metadataArray->AppendObject(metadata)",
                ]),
            ];
            if let Some(r) = st.range {
                if wcsicmp(st.name, "ToolsLogLevel") != std::cmp::Ordering::Equal {
                    let text = Some(range_text(r));
                    entries.push(("Range", text, [1633, 1634, 1635, 1636], [
                        "XbtpCreateJsonObject(&metadata)",
                        "metadata->SetString(SettingMetadataNameProperty, ConsoleConfig::g_pszMetadataRange)",
                        "metadata->SetString(SettingMetadataValueProperty, strRange)",
                        "metadataArray->AppendObject(metadata)",
                    ]));
                }
            }
            for (name, val, lines, exprs) in entries {
                let mut m = Com::null();
                let hr = xbtp::create_json_object(m.put());
                if hr < 0 {
                    op.fail(hr, lines[0], exprs[0]);
                    break 'done;
                }
                let hr = set_string_text(m.get(), "Name", name);
                if hr < 0 {
                    op.fail(hr, lines[1], exprs[1]);
                    break 'done;
                }
                let vb = match &val {
                    Some(v) => Bstr::from_str(v),
                    None => Bstr::null(),
                };
                let hr = set_string(m.get(), "Value", vb.ptr());
                if hr < 0 {
                    op.fail(hr, lines[2], exprs[2]);
                    break 'done;
                }
                let hr = array::append_object(md.get(), m.get());
                if hr < 0 {
                    op.fail(hr, lines[3], exprs[3]);
                    break 'done;
                }
            }
            let n = w("Metadata");
            let hr = json::set_array(setting.get(), n.as_ptr(), md.get());
            if hr < 0 {
                op.fail(hr, 1640, "setting->SetArray(SettingMetadataArrayProperty, metadataArray)");
                break 'done;
            }
            let hr = array::append_object(settings.get(), setting.get());
            if hr < 0 {
                op.fail(hr, 1642, "settingsArray->AppendObject(setting)");
                break 'done;
            }
        }
        let n = w("SettingsMetadata");
        let hr = json::set_array(settings_body.get(), n.as_ptr(), settings.get());
        if hr < 0 {
            op.fail(hr, 1645, "settingsBody->SetArray(SettingsMetadataArrayProperty, settingsArray)");
            break 'done;
        }
        let n = w("SettingsMetadataArrayObject");
        let hr = json::set_object(body.get(), n.as_ptr(), settings_body.get());
        if hr < 0 {
            op.fail(hr, 1646, "replyBody->SetObject(SettingsMetadataArrayObjectProperty, settingsBody)");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 1647, "XtfSerializeJsonBody(replyBody, pReply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    op.complete(reply, body.get());
    op.finish(activity)
}

pub unsafe extern "system" fn get_settings_ex_handler(_session: usize, _request: usize, reply: usize) -> HRESULT {
    let activity = Activity::start(b"XtfConsoleControl: GetSettingsExHandler\0");
    let mut op = Op::new(CONFIG_CPP, "GetSettingsExHandler", reply);
    let mut body = Com::null();
    let mut settings = Com::null();
    let value = Bstr::null();
    'done: {
        let list = sorted_list();
        let hr = xbtp::create_json_object(body.put());
        if hr < 0 {
            op.fail(hr, 1681, "XbtpCreateJsonObject(&replyBody)");
            break 'done;
        }
        let hr = xbtp::create_json_array(settings.put());
        if hr < 0 {
            op.fail(hr, 1682, "XbtpCreateJsonArray(&settingsArray)");
            break 'done;
        }
        for st in list {
            let mut setting = Com::null();
            let hr = xbtp::create_json_object(setting.put());
            if hr < 0 {
                op.fail(hr, 1689, "XbtpCreateJsonObject(&setting)");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "Name", st.name);
            if hr < 0 {
                op.fail(hr, 1691, "setting->SetString(ConsoleGetSettingsExProperty_Name, settingMetadata.pszName)");
                break 'done;
            }
            let hr = set_string(setting.get(), "Value", value.ptr());
            if hr < 0 {
                op.fail(hr, 1692, "setting->SetString(ConsoleGetSettingsExProperty_Value, strValue)");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "Category", st.category);
            if hr < 0 {
                op.fail(hr, 1693, "setting->SetString(ConsoleGetSettingsExProperty_Category, settingMetadata.pszCategory)");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "RebootRequired", tf(st.reboot_required));
            if hr < 0 {
                op.fail(hr, 1696, "setting->SetString( ConsoleGetSettingsExProperty_RebootRequired, (settingMetadata.fRebootRequired == false ? L\"false\" : L\"true\"))");
                break 'done;
            }
            let hr = set_string_text(setting.get(), "Hidden", tf(st.hidden));
            if hr < 0 {
                op.fail(hr, 1699, "setting->SetString( ConsoleGetSettingsExProperty_Hidden, (settingMetadata.fHidden == false ? L\"false\" : L\"true\"))");
                break 'done;
            }
            if let Some(r) = st.range {
                let mut ra = Com::null();
                let hr = xbtp::create_json_array(ra.put());
                if hr < 0 {
                    op.fail(hr, 1703, "XbtpCreateJsonArray(&rangeArray)");
                    break 'done;
                }
                for item in r {
                    let t = w(item);
                    let hr = array::append_string(ra.get(), t.as_ptr());
                    if hr < 0 {
                        op.fail(hr, 1706, "rangeArray->AppendString(settingMetadata.rangeArr[j])");
                        break 'done;
                    }
                }
                let n = w("Range");
                let hr = json::set_array(setting.get(), n.as_ptr(), ra.get());
                if hr < 0 {
                    op.fail(hr, 1708, "setting->SetArray(ConsoleGetSettingsExProperty_Range, rangeArray)");
                    break 'done;
                }
            }
            let hr = array::append_object(settings.get(), setting.get());
            if hr < 0 {
                op.fail(hr, 1711, "settingsArray->AppendObject(setting)");
                break 'done;
            }
        }
        let n = w("Settings");
        let hr = json::set_array(body.get(), n.as_ptr(), settings.get());
        if hr < 0 {
            op.fail(hr, 1714, "replyBody->SetArray(ConsoleGetSettingsExProperty_Settings, settingsArray)");
            break 'done;
        }
        let hr = serialize_end_body(body.get(), reply as Obj);
        if hr < 0 {
            op.fail(hr, 1715, "XtfSerializeJsonBody(replyBody, pReply)");
            break 'done;
        }
    }
    if op.hr < 0 {
        op.xce_empty();
    }
    op.complete(reply, body.get());
    op.finish(activity)
}
