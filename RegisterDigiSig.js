/* 
 * To change this license header, choose License Headers in Project Properties.
 * To change this template file, choose Tools | Templates
 * and open the template in the editor.
 */
$(document).ready(function ()
{
    $('#nameAsPerAadhaar').keyup(function () {
        this.value = this.value.toUpperCase();
    });
    $('#nameAsPerAadhaar').blur(function () {
        this.value = this.value.trim().toUpperCase();
    });
     $('#eSignDesignation').keyup(function () {
        this.value = this.value.toUpperCase();
    });
    $('#eSignDesignation').blur(function () {
        this.value = this.value.trim().toUpperCase();
    });
    
});
function validateName() {
    var msg = '';
    msg = validate_required(document.getElementById("name"), 'Enter Name of Authorized Signatory. ', 'nameWithApostrophe', "Enter valid Name. ");
    if ($("#name").val().length > 120)
    {
        msg += 'Name of Authorized Signatory can contain only 120 characters.';
    }

    setAndRemoveErrorMsg($("#name"), msg);

    return msg;
}
function validateDesignation() {
    var msg = '';
    msg = validate_required(document.getElementById("designation"), 'Enter Designation.', 'p', "Enter valid Designation cannot contain <, >, ' spl. characters");
    if ($("#designation").val().length > 120) {
        msg += 'Designation can contain only 120 characters.';
    }
    setAndRemoveErrorMsg($("#designation"), msg);

    return msg;
}
function validateMobileNumber() {
    var msg = '';
    msg = validate_required(document.getElementById("mobileNumber"), 'Enter mobile number. ', 'd', "Enter valid mobile number. ");
    if ($("#mobileNumber").val().length != 10) {
        msg += 'Mobile number should be 10 digits. ';
    }
    setAndRemoveErrorMsg($("#mobileNumber"), msg);
    return msg;
}
function validateRadioButton() {
    var msg = '';
    if ($('input[name=typeOfSign]:checked').length) {
        msg += 'Select option for certificate registration.';
    }
    setAndRemoveErrorMsg($("#typeOfSign1"), msg);
    return msg;
}
function validateDetails() {

    var sucMsg = true;
    var msg = "";
    msg += validateName();
    msg += validateDesignation();
    msg += validateMobileNumber();
    if (msg !== "") {
        sucMsg = false;
    }
    return sucMsg;
}

function escapeSelector(s) {
    return s.replace(/(:|\.|\[|\])/g, "\\$1");
}

function validateAadhaar() {
    var msg = '';
    msg = validate_required(document.getElementById("aadhaar"), 'Enter Aadhaar. ', 'd1', "Enter valid Aadhaar. ");
    if ($("#aadhaar").val().length !== 12)
    {
        msg += 'Aadhaar should be 12 digits. ';
    }

    setAndRemoveErrorMsg($("#aadhaar"), msg);
    return msg;
}
function validateNameAsPerAadhaar() {
    var msg = '';
    msg = validate_required(document.getElementById("nameAsPerAadhaar"), 'Enter Name as per Aadhaar.', 'p', "Enter valid Name.");
    if ($("#nameAsPerAadhaar").val().length > 120) {
        msg += 'Name as per Aadhaar can contain only 120 characters.';
    }
    setAndRemoveErrorMsg($("#nameAsPerAadhaar"), msg);
    return msg;
}
function validateESignDesignation() {
    var msg = '';
    msg = validate_required(document.getElementById("eSignDesignation"), 'Enter Designation.', 'p', "Enter valid Designation cannot contain <, >, ' spl. characters");
    if ($("#eSignDesignation").val().length > 120) {
        msg += 'Designation can contain only 120 characters.';
    }
    setAndRemoveErrorMsg($("#eSignDesignation"), msg);

    return msg;
}
function validateGender() {
    var msg = '';
    var fvalue = $("#genderCode").val().trim();
    var regex = "M|F|T";
    if (fvalue == null || fvalue == "") {
        msg += 'Select Gender';
    }
    if (fvalue.length > 0)
    {
        var re = new RegExp(regex);
        var str = fvalue.match(re);
        if (str == null && str == "")
            msg += 'Enter Valid Gender';
    }
    setAndRemoveErrorMsg($("#genderCode"), msg);
    return msg;
}
function validateDOB() {
    var msg = '';
    var fvalue = $("#dob").val().trim();
    var regex = "(0?[1-9]|[12][0-9]|3[01])/(0?[1-9]|1[012])/((19|20)\\d\\d)";
    if (fvalue == null || fvalue == "") {
        msg += 'Select Date of Birth';
    }
    if (fvalue.length > 0)
    {
        var re = new RegExp(regex);
        var str = fvalue.match(re);
        if (str == null && str == "")
            msg += 'Enter Valid Date';
    }

    setAndRemoveErrorMsg1($("#dobErr"), $("#dob"), msg);
    return msg;
}

function setAndRemoveErrorMsg1(object1, object2, msg) {
    if (msg !== '') {
        $(object1).parent().find("P").remove();
        $(object1).parent().append('<p style="color:red;">' + msg + '</p>');
        object2.focus(function () {
            $(object1).parent().find("P").remove();
        });
    }
}

function validateOTP() {
    var msg = '';
    msg = validate_required(document.getElementById("otp"), 'Enter OTP.', 'pin', "Enter valid OTP.");
    setAndRemoveErrorMsg($("#otp"), msg);
    return msg;
}
function validateESignDetails() {
    
    if (!$("#aadhaarConsentChkBox").is(":checked")) {
        alert("Please provide your consent.");
        return false;
    }

    var sucMsg = true;
    var msg = "";
    msg += validateAadhaar();
    msg += validateNameAsPerAadhaar();
    msg += validateESignDesignation();
    msg += validateGender();
    msg += validateDOB();
    if (otpGot == 'Y')
        msg += validateOTP();
    if (msg !== "") {
        sucMsg = false;
    }
    return sucMsg;
}
 function onlyAlphabets(e, t) {
                try {
                    if (window.event) {
                        var charCode = window.event.keyCode;
                    }
                    else if (e) {
                        var charCode = e.which;
                    }
                    else { return true; }
                    if ((charCode > 64 && charCode < 91) || (charCode > 96 && charCode < 123) || charCode == 8)
                        return true;
                    else
                        return false;
                }
                catch (err) {
                    alert(err.Description);
                }
            }
            function isNumber(evt) {
                evt = (evt) ? evt : window.event;
                var charCode = (evt.which) ? evt.which : evt.keyCode;
                if (charCode > 31 && (charCode < 48 || charCode > 57)) {
                    return false;
                }
                return true;
            }
 function changePsBlockUI(message)
{
    $.blockUI({css: {
            border: 'none',
            padding: '2px',
            backgroundColor: '#000',
            '-webkit-border-radius': '5px',
            '-moz-border-radius': '5px',
            opacity: .5,
            color: '#fff'
        },
        message: "<span><i class='fa fa-cog fa-3x fa-spin'></i><h5>" + message + "</h5></span>"});
}
function getCertificatefromService() {
    changePsBlockUI('Fetching local certificates.');
    $.ajax({
        type: "POST",
        url: "https://dsc.epfindia.gov.in:60015/dscapi/getCertificateList/",
        data: dscRegistrationReq,
        contentType: false,
        async: true,
        headers: {"api-key": apiKey,
            "client-id": clientId},
        success: function (result) {
            $.unblockUI();
            if (result === null || result === undefined || result === '') {
                alert("Error occured while fetching certificate details. Please retry.");
            } else {
                verifyCertificateList(result);
            }
        },
        error: function (result, status, error) {
            console.log(result.responseText.toString());
            alert("Error occured while connecting DSC-Utility application, Please check. \n\If error persists then restart the application.");
            $.unblockUI();
        }

    });
}
function verifyCertificateList(result) {
    if (validateDetails()) {
        var name = $('#name').val();
        var signingDetailsJson = {
            "certificateList": result,
            "signatoryName": name
        };
        var signingDetails = JSON.stringify(signingDetailsJson);
        changePsBlockUI('Verifying certificates, Please wait.');
        $.ajax({
            type: "POST",
            url: getCertificateList,
            data: signingDetails,
            contentType: "application/json; charset=utf-8",
            async: true,
            success: function (result) {
                $.unblockUI();
                if (result.err !== undefined) {
                    alert(result.err);
                } else {
                    $('#appletContainer').html(result);
                    $('#appletModal').modal('show');
                    $('#appletModal').modal({backdrop: "false"});
                }
            }, error: function (result, status, error) {
                console.log(result.responseText.toString());
                alert("Unexpected error occurred. Please try again.");
                $.unblockUI();
            }
        });
    } else {
        return false;
    }
}
$(document).ready(function () {
    $("#digitalSignatureRegistration").submit(function () {
        if (validateDetails()) {
            getCertificates();
        } else {
            return false;
        }
    });

    $("#eSignRegistration").submit(function () {
        return validateESignDetails();
    });

    $('[data-type="date"]').datepicker( 
            {dateFormat: "dd/mm/yy",
                changeMonth: true,
                changeYear: true,
                maxDate: new Date(),
                yearRange: "1900:" + new Date().getFullYear()
                        /*onSelect: function ()
                         {
                         $(this).focus();
                         $(this).change();
                         }*/
            });



    $("i.fa.fa-calendar").click(function () {
        var dataTarget = "#" + escapeSelector($(this).attr("data-bs-target"));
        $(dataTarget).datepicker("show");
    });

//    $("#errMessage").fadeTo(3000, 1).slideUp(2000, function () {
//        $("#errMessage").slideUp(2000);
//    });
//
//    $("#errorMessage").fadeTo(3000, 1).slideUp(2000, function () {
//        $("#errorMessage").slideUp(2000);
//    });
//

});
