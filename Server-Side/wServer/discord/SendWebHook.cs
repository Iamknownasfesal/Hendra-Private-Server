using System;
using System.Collections.Generic;
using System.Linq;
using System.Text;
using System.Threading.Tasks;
using System.Net;
using System.Collections.Specialized;
using System.Net.Http;
using Newtonsoft.Json;

namespace wServer.discord
{
    class SendWebHook
    {
        private static HttpClient client;
        public static HttpClient Client
        {
            get
            {
                if (client == null)
                    client = new HttpClient();

                return client;
            }
        }

        private static object successWebHook;

        public static object GetSuccessWebHook()
        {
            return successWebHook;
        }

        private static void SetSuccessWebHook(object value)
        {
            successWebHook = value;
        }

        public static void Post(string selectedTitle, string selectedDescription, string name, int selectedType, string whoBanned = "NaN",string reason = "NaN",string ipAdress = "NaN", string killername = "NaN", string maxed = "NaN", string finalFame = "NaN")
        {
            var WebHookId = "699237174763061299";
            var WebHookToken = "t1kMS4FHqCzaOzSzdBnbg5EmfAq7uyservL5R3qsKOK-y9eJtA5eUliQlHOfVgCwhRko";
            const string colorRed = "E7421F";


            if (selectedType == 0)
            {
                SetSuccessWebHook(new
                {
                    username = "Brainless Logger",
                    embeds = new List<object>
                {
                    new
                    {
                        title = selectedTitle,
                        description = selectedDescription,
                        color= int.Parse(colorRed, System.Globalization.NumberStyles.HexNumber),
                        fields= new List<object>
                        {
                            new
                            {
                                name = "Name of Player",
                                value = name
                            },
                            new
                            {
                                name = "Died to",
                                value = killername
                            },
                            new
                            {
                                name = "Maxed Stats",
                                value = maxed + "/8"
                            },
                            new
                            {
                                name = "Final Fame of Char",
                                value = finalFame
                            }
                        }
                    }
                }
                });
            }
            else if (selectedType == 1)
            {
                SetSuccessWebHook(new
                {
                    username = "Brainless Logger",
                    embeds = new List<object>
                {
                    new
                    {
                        title = selectedTitle,
                        description = selectedDescription,
                        color= int.Parse(colorRed, System.Globalization.NumberStyles.HexNumber),
                        fields= new List<object>
                        {
                            new
                            {
                                name = "Name of Player",
                                value = name
                            },
                            new
                            {
                                name = "Reason",
                                value = reason
                            },
                            new
                            {
                                name = "Who Banned the Player",
                                value = whoBanned
                            }
                        }
                    }
                }
                });
            }
            else if (selectedType == 2)
            {
                SetSuccessWebHook(new
                {
                    username = "Brainless Logger",
                    embeds = new List<object>
                {
                    new
                    {
                        title = selectedTitle,
                        description = selectedDescription,
                        color= int.Parse(colorRed, System.Globalization.NumberStyles.HexNumber),
                        fields= new List<object>
                        {
                            new
                            {
                                name = "Name of Player",
                                value = name
                            },
                            new
                            {
                                name = "Reason",
                                value = reason
                            },
                            new
                            {
                                name = "IP Adress of Player",
                                value = ipAdress
                            },
                            new
                            {
                                name = "Who Banned the Player",
                                value = whoBanned
                            }
                        }
                    }
                }
                });
            }
            else if (selectedType == 3)
            {
                SetSuccessWebHook(new
                {
                    username = "Brainless Logger",
                    embeds = new List<object>
                {
                    new
                    {
                        title = selectedTitle,
                        description = selectedDescription,
                        color= int.Parse(colorRed, System.Globalization.NumberStyles.HexNumber),
                        fields= new List<object>
                        {
                            new
                            {
                                name = "Name of Player",
                                value = name
                            },
                            new
                            {
                                name = "IP Adress of Player",
                                value = ipAdress
                            },
                            new
                            {
                                name = "Who Unbanned the Player",
                                value = whoBanned
                            }
                        }
                    }
                }
                });
            }
            else
            {
                return;
            }

            string EndPoint = string.Format("https://discordapp.com/api/webhooks/{0}/{1}", WebHookId, WebHookToken);
            var content = new StringContent(JsonConvert.SerializeObject(GetSuccessWebHook()), Encoding.UTF8, "application/json");

            Client.PostAsync(EndPoint, content).Wait();
        }

    }
}
